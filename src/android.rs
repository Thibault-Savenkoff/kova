// An Android app, made by Kova alone (PLAN Part 4 "Android"): no SDK, no Java. An `.apk` is a zip
// file holding the app's description (`AndroidManifest.xml`, in Android's binary form) and the
// program as a library for each processor, with a signature after the files.
//
// The signature is scheme v2 (`signature`), made with a key of the developer's kept in their
// configuration folder (`key`): Android refuses to update an app signed with another key.

use std::path::PathBuf;

use p256::ecdsa::signature::Signer;
use p256::ecdsa::{DerSignature, SigningKey};

use crate::crypto::sha256_of;

/// Android's oldest version running these apps: 8.0, 2017.
const MIN_SDK: u32 = 26;
const TARGET_SDK: u32 = 35;

/// Android's name for a processor.
pub fn abi(arm: bool) -> &'static str {
    if arm { "arm64-v8a" } else { "x86_64" }
}

/// The `.apk` of a program named `title`: one library per processor (Android's name, its bytes).
pub fn apk(title: &str, libraries: &[(&str, Vec<u8>)]) -> Result<Vec<u8>, String> {
    let manifest = manifest(&format!("dev.kova.app.{}", package_word(title)), title);
    let mut files: Vec<(String, &[u8])> = vec![("AndroidManifest.xml".to_string(), &manifest)];
    for (abi, library) in libraries {
        files.push((format!("lib/{abi}/libkova.so"), library));
    }
    let zip = zip(&files);
    signed(zip, &key()?)
}

/// `title` as a word of a package's name.
fn package_word(title: &str) -> String {
    let word: String = title.chars().filter(|c| c.is_ascii_alphanumeric()).collect::<String>().to_lowercase();
    if word.starts_with(|c: char| c.is_ascii_alphabetic()) { word } else { format!("app{word}") }
}

// ---- The manifest: XML in Android's binary form ----

enum Value<'a> {
    Text(&'a str),
    Number(u32),
    Bool(bool),
}

use Value::{Bool, Number, Text};

/// An XML element: its name, its attributes (name, value), what it holds.
struct Element<'a>(&'a str, Vec<(&'a str, Value<'a>)>, Vec<Element<'a>>);

/// The attributes of the `android:` namespace, with the numbers Android knows them by.
const ATTRIBUTES: [(&str, u32); 11] = [
    ("label", 0x0101_0001),
    ("name", 0x0101_0003),
    ("hasCode", 0x0101_000c),
    ("exported", 0x0101_0010),
    ("configChanges", 0x0101_001f),
    ("value", 0x0101_0024),
    ("minSdkVersion", 0x0101_020c),
    ("versionCode", 0x0101_021b),
    ("versionName", 0x0101_021c),
    ("targetSdkVersion", 0x0101_0270),
    ("extractNativeLibs", 0x0101_04ea),
];

const NAMESPACE: &str = "http://schemas.android.com/apk/res/android";
const NONE: u32 = u32::MAX;

fn manifest(package: &str, title: &str) -> Vec<u8> {
    let category = |name| Element("category", vec![("name", Text(name))], vec![]);
    let action = |name| Element("action", vec![("name", Text(name))], vec![]);
    let root = Element(
        "manifest",
        vec![("versionCode", Number(1)), ("versionName", Text("1.0")), ("package", Text(package))],
        vec![
            Element("uses-sdk", vec![("minSdkVersion", Number(MIN_SDK)), ("targetSdkVersion", Number(TARGET_SDK))], vec![]),
            Element("uses-permission", vec![("name", Text("android.permission.INTERNET"))], vec![]),
            Element(
                "application",
                vec![("label", Text(title)), ("hasCode", Bool(false)), ("extractNativeLibs", Bool(false))],
                vec![Element(
                    "activity",
                    vec![
                        ("label", Text(title)),
                        ("name", Text("android.app.NativeActivity")),
                        ("exported", Bool(true)),
                        // Turning the phone does not restart the program.
                        ("configChanges", Number(0x4000_3fff)),
                    ],
                    vec![
                        Element("meta-data", vec![("name", Text("android.app.lib_name")), ("value", Text("kova"))], vec![]),
                        Element("intent-filter", vec![], vec![action("android.intent.action.MAIN"), category("android.intent.category.LAUNCHER")]),
                    ],
                )],
            ),
        ],
    );
    binary_xml(&root)
}

/// One chunk of binary XML: its type, the size of its header, its size, then what follows.
fn chunk(kind: u16, header: u16, body: &[u8]) -> Vec<u8> {
    let mut c = vec![];
    c.extend(kind.to_le_bytes());
    c.extend(header.to_le_bytes());
    c.extend((body.len() as u32 + 8).to_le_bytes());
    c.extend(body);
    c
}

fn words(v: &[u32]) -> Vec<u8> {
    v.iter().flat_map(|w| w.to_le_bytes()).collect()
}

fn binary_xml(root: &Element) -> Vec<u8> {
    // The names Android must recognize come first: the table after this one numbers them.
    let mut strings: Vec<&str> = ATTRIBUTES.iter().map(|(name, _)| *name).collect();
    let known = strings.len();
    strings.extend(["android", NAMESPACE]);
    fn collect<'a>(e: &Element<'a>, strings: &mut Vec<&'a str>) {
        let mut add = |s: &'a str| {
            if !strings.contains(&s) {
                strings.push(s)
            }
        };
        add(e.0);
        for (name, value) in &e.1 {
            add(name);
            if let Text(t) = value {
                add(t);
            }
        }
        e.2.iter().for_each(|child| collect(child, strings));
    }
    collect(root, &mut strings);
    let index = |s: &str| strings.iter().position(|t| *t == s).unwrap() as u32;

    // The texts, in UTF-16.
    let (mut offsets, mut texts) = (vec![], vec![]);
    for s in &strings {
        offsets.push(texts.len() as u32);
        let units: Vec<u16> = s.encode_utf16().collect();
        texts.extend((units.len() as u16).to_le_bytes());
        units.iter().for_each(|u| texts.extend(u.to_le_bytes()));
        texts.extend([0, 0]);
    }
    texts.resize(texts.len().next_multiple_of(4), 0);
    let n = strings.len() as u32;
    let mut pool = words(&[n, 0, 0, 28 + 4 * n, 0]);
    pool.extend(words(&offsets));
    pool.extend(texts);
    let mut body = chunk(0x0001, 28, &pool);
    let numbers: Vec<u32> = ATTRIBUTES.iter().map(|(_, n)| *n).collect();
    assert_eq!(known, numbers.len());
    body.extend(chunk(0x0180, 8, &words(&numbers)));

    let (prefix, uri) = (index("android"), index(NAMESPACE));
    let namespace = words(&[1, NONE, prefix, uri]);
    body.extend(chunk(0x0100, 16, &namespace));
    fn write(e: &Element, index: &dyn Fn(&str) -> u32, uri: u32, out: &mut Vec<u8>) {
        // Sorted by Android's number; attributes it does not number last.
        let number = |name: &str| ATTRIBUTES.iter().find(|(n, _)| *n == name).map_or(u32::MAX, |(_, n)| *n);
        let mut attributes: Vec<_> = e.1.iter().collect();
        attributes.sort_by_key(|(name, _)| number(name));
        let mut start = words(&[1, NONE, NONE, index(e.0)]);
        for v in [0x14u16, 0x14, attributes.len() as u16, 0, 0, 0] {
            start.extend(v.to_le_bytes());
        }
        for (name, value) in attributes {
            let (raw, kind, data) = match value {
                Text(t) => (index(t), 0x03u8, index(t)),
                Number(n) => (NONE, 0x10, *n),
                Bool(b) => (NONE, 0x12, if *b { u32::MAX } else { 0 }),
            };
            let space = if number(name) == u32::MAX { NONE } else { uri };
            start.extend(words(&[space, index(name), raw]));
            start.extend([8, 0, 0, kind]);
            start.extend(data.to_le_bytes());
        }
        out.extend(chunk(0x0102, 16, &start));
        e.2.iter().for_each(|child| write(child, index, uri, out));
        out.extend(chunk(0x0103, 16, &words(&[1, NONE, NONE, index(e.0)])));
    }
    write(root, &index, uri, &mut body);
    body.extend(chunk(0x0101, 16, &namespace));
    chunk(0x0003, 8, &body)
}

// ---- The zip file ----

fn crc32(data: &[u8]) -> u32 {
    let mut crc = u32::MAX;
    for &b in data {
        crc ^= b as u32;
        for _ in 0..8 {
            crc = crc >> 1 ^ 0xEDB8_8320 & (crc & 1).wrapping_neg();
        }
    }
    !crc
}

/// A zip file: (entries, central directory, the end record), so that the signature can go between
/// the first two. Nothing is compressed, and each library starts on a page of 16 KB: Android
/// maps it straight from the file.
struct Zip {
    entries: Vec<u8>,
    directory: Vec<u8>,
    end: Vec<u8>,
}

fn zip(files: &[(String, &[u8])]) -> Zip {
    const PAGE: usize = 16384;
    let (mut entries, mut directory) = (vec![], vec![]);
    for (name, data) in files {
        let at = entries.len() as u32;
        let (crc, size) = (crc32(data), data.len() as u32);
        // Version 10, no flags, stored, time and date 0; then the CRC, the sizes, the name's length.
        let mut common = vec![];
        for v in [10u16, 0, 0, 0, 0x21] {
            common.extend(v.to_le_bytes());
        }
        common.extend(words(&[crc, size, size]));
        common.extend((name.len() as u16).to_le_bytes());
        // The room before the data, as `zipalign` makes it.
        let alignment = if name.ends_with(".so") { PAGE } else { 4 };
        let before = entries.len() + 30 + name.len() + 6;
        let padding = before.next_multiple_of(alignment) - before + 2;
        let mut extra = vec![];
        extra.extend(0xD935u16.to_le_bytes());
        extra.extend((padding as u16).to_le_bytes());
        extra.extend((alignment as u16).to_le_bytes());
        extra.resize(4 + padding, 0);
        entries.extend(b"PK\x03\x04");
        entries.extend(&common);
        entries.extend((extra.len() as u16).to_le_bytes());
        entries.extend(name.as_bytes());
        entries.extend(&extra);
        entries.extend(*data);

        directory.extend(b"PK\x01\x02");
        directory.extend(10u16.to_le_bytes());
        directory.extend(&common);
        // No extra field, comment, disk, attributes; then where the entry is.
        directory.extend([0; 12]);
        directory.extend(at.to_le_bytes());
        directory.extend(name.as_bytes());
    }
    let mut end = b"PK\x05\x06\0\0\0\0".to_vec();
    for _ in 0..2 {
        end.extend((files.len() as u16).to_le_bytes());
    }
    end.extend(words(&[directory.len() as u32, entries.len() as u32]));
    end.extend([0, 0]);
    Zip { entries, directory, end }
}

// ---- The signature ----

/// `pieces` after their total length on 4 bytes.
fn prefixed(pieces: &[&[u8]]) -> Vec<u8> {
    let mut out = (pieces.iter().map(|p| p.len()).sum::<usize>() as u32).to_le_bytes().to_vec();
    pieces.iter().for_each(|p| out.extend(*p));
    out
}

/// ECDSA with SHA-256, in the signature scheme's numbers.
const ALGORITHM: u32 = 0x0201;

/// APK Signature Scheme v2: a block between the files and the directory, with the digest of
/// everything else, by pieces of 1 MB.
fn signed(zip: Zip, key: &SigningKey) -> Result<Vec<u8>, String> {
    let mut digests = vec![];
    let mut count = 0u32;
    for section in [&zip.entries, &zip.directory, &zip.end] {
        for piece in section.chunks(1 << 20) {
            digests.extend(sha256_of(&[&[0xA5], &(piece.len() as u32).to_le_bytes(), piece]));
            count += 1;
        }
    }
    let digest = sha256_of(&[&[0x5A], &count.to_le_bytes(), &digests]);

    let public = public_key(key);
    let certificate = certificate(key, &public);
    let algorithm = ALGORITHM.to_le_bytes();
    let digest_entry = prefixed(&[&algorithm, &prefixed(&[&digest])]);
    let signed_data = prefixed(&[
        &prefixed(&[&digest_entry]),
        &prefixed(&[&prefixed(&[&certificate])]),
        &prefixed(&[]),
    ]);
    let signature: DerSignature = key.sign(&signed_data[4..]);
    let signature_entry = prefixed(&[&algorithm, &prefixed(&[signature.as_bytes()])]);
    let signer = prefixed(&[&signed_data, &prefixed(&[&signature_entry]), &prefixed(&[&public])]);
    let value = prefixed(&[&signer]);

    let pair_size = 4 + value.len() as u64;
    let block_size = 8 + pair_size + 8 + 16;
    let mut block = block_size.to_le_bytes().to_vec();
    block.extend(pair_size.to_le_bytes());
    block.extend(0x7109_871Au32.to_le_bytes());
    block.extend(&value);
    block.extend(block_size.to_le_bytes());
    block.extend(b"APK Sig Block 42");

    // The directory now starts after the block; the digest was made without knowing it.
    let mut end = zip.end;
    let at = (zip.entries.len() + block.len()) as u32;
    end[16..20].copy_from_slice(&at.to_le_bytes());
    Ok([zip.entries, block, zip.directory, end].concat())
}

/// A length in DER: what follows a type byte.
fn der(kind: u8, pieces: &[&[u8]]) -> Vec<u8> {
    let size: usize = pieces.iter().map(|p| p.len()).sum();
    let mut out = vec![kind];
    match size {
        0..=127 => out.push(size as u8),
        128..=255 => out.extend([0x81, size as u8]),
        _ => out.extend([0x82, (size >> 8) as u8, size as u8]),
    }
    pieces.iter().for_each(|p| out.extend(*p));
    out
}

const SEQUENCE: u8 = 0x30;
/// The object identifiers: ecdsa-with-SHA256, id-ecPublicKey, prime256v1, commonName.
const ECDSA_SHA256: [u8; 10] = [0x06, 0x08, 0x2A, 0x86, 0x48, 0xCE, 0x3D, 0x04, 0x03, 0x02];
const EC_PUBLIC_KEY: [u8; 9] = [0x06, 0x07, 0x2A, 0x86, 0x48, 0xCE, 0x3D, 0x02, 0x01];
const PRIME256V1: [u8; 10] = [0x06, 0x08, 0x2A, 0x86, 0x48, 0xCE, 0x3D, 0x03, 0x01, 0x07];
const COMMON_NAME: [u8; 5] = [0x06, 0x03, 0x55, 0x04, 0x03];

/// The public key, as a certificate holds it.
fn public_key(key: &SigningKey) -> Vec<u8> {
    let point = key.verifying_key().to_encoded_point(false);
    der(SEQUENCE, &[&der(SEQUENCE, &[&EC_PUBLIC_KEY, &PRIME256V1]), &der(0x03, &[&[0], point.as_bytes()])])
}

/// A certificate signed by the key itself: Android asks nobody to vouch for an app, it only checks
/// that updates come from the same key. It is the same at every build.
fn certificate(key: &SigningKey, public: &[u8]) -> Vec<u8> {
    let algorithm = der(SEQUENCE, &[&ECDSA_SHA256]);
    let name = der(SEQUENCE, &[&der(0x31, &[&der(SEQUENCE, &[&COMMON_NAME, &der(0x0C, &[b"Kova developer"])])])]);
    let validity = der(SEQUENCE, &[&der(0x17, &[b"200101000000Z"]), &der(0x18, &[b"20991231235959Z"])]);
    let serial = der(0x02, &[&[1]]);
    let version = der(0xA0, &[&der(0x02, &[&[2]])]);
    let body = der(SEQUENCE, &[&version, &serial, &algorithm, &name, &validity, &name, public]);
    let signature: DerSignature = key.sign(&body);
    der(SEQUENCE, &[&body, &algorithm, &der(0x03, &[&[0], signature.as_bytes()])])
}

/// Where the developer's key is: it must outlive caches, and stay out of any project.
fn key_file() -> Result<PathBuf, String> {
    let env = |name| std::env::var_os(name).filter(|v| !v.is_empty()).map(PathBuf::from);
    if let Some(file) = env("KOVA_ANDROID_KEY") {
        return Ok(file);
    }
    let folder = if cfg!(windows) {
        env("APPDATA")
    } else {
        env("XDG_CONFIG_HOME").or_else(|| env("HOME").map(|h| h.join(".config")))
    };
    Ok(folder.ok_or("no home folder for the Android key")?.join("kova").join("android.key"))
}

/// The developer's key, made the first time. Readable by them alone.
fn key() -> Result<SigningKey, String> {
    let file = key_file()?;
    let say = |e: std::io::Error| format!("{}: {e}", file.display());
    if file.exists() {
        let secret = std::fs::read(&file).map_err(say)?;
        return SigningKey::from_slice(&secret).map_err(|e| format!("{}: {e}", file.display()));
    }
    if let Some(folder) = file.parent() {
        std::fs::create_dir_all(folder).map_err(say)?;
    }
    let key = loop {
        let mut secret = [0; 32];
        crate::interp::chance(&mut secret);
        if let Ok(key) = SigningKey::from_slice(&secret) {
            break key;
        }
    };
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    use std::io::Write;
    options.open(&file).and_then(|mut f| f.write_all(&key.to_bytes())).map_err(say)?;
    Ok(key)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crypto::sha256;

    #[test]
    fn crc_of_a_known_text() {
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
    }

    #[test]
    fn signature_checks_with_the_key() {
        use p256::ecdsa::signature::Verifier;
        let key = SigningKey::from_slice(&sha256(b"test")).unwrap();
        let signature: DerSignature = key.sign(b"message");
        assert!(key.verifying_key().verify(b"message", &signature).is_ok());
    }

    #[test]
    fn zip_points_at_its_directory() {
        let files = [("a.txt".to_string(), &b"hello"[..])];
        let apk = signed(zip(&files), &SigningKey::from_slice(&sha256(b"test")).unwrap()).unwrap();
        let end = apk.len() - 22;
        assert_eq!(&apk[end..end + 4], b"PK\x05\x06");
        let at = u32::from_le_bytes(apk[end + 16..end + 20].try_into().unwrap()) as usize;
        assert_eq!(&apk[at..at + 4], b"PK\x01\x02");
        assert_eq!(&apk[at - 16..at], b"APK Sig Block 42");
    }
}

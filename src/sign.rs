//! Signing an iPhone app (PLAN Part 4 "iOS"): what Apple's `codesign` does, written out so that a
//! Linux or Windows PC can do it too. Given a provisioning profile (made once by Xcode) and the
//! developer's identity (private key + certificate, as a PEM file), `bundle` turns the unsigned
//! `Name.app` folder that `src/ios.rs` wrote into one an iPhone accepts:
//!
//! * `embedded.mobileprovision`: the profile, copied;
//! * `_CodeSignature/CodeResources`: the hashes of the files around the executable;
//! * in the executable, replacing the ad-hoc signature of `link.rs`: a code directory (the hash of
//!   each page and of the small documents around it), the requirements, the entitlements twice
//!   (XML and DER) and a CMS signature made with the developer's RSA key.
//!
//! The layouts were read from an app signed by `codesign` and the result is checked against it.

use crate::crypto::{base64, from_base64, sha256};
use crate::t;
use rsa::pkcs8::DecodePrivateKey;
use rsa::pkcs8::DecodePublicKey;
use rsa::signature::{SignatureEncoding, Signer};
use rsa::traits::PublicKeyParts;
use rsa::RsaPrivateKey;
use std::path::Path;
use std::sync::OnceLock;

/// `--profile` and `--identity` of `kova build --target ios`.
pub static PROFILE: OnceLock<String> = OnceLock::new();
pub static IDENTITY: OnceLock<String> = OnceLock::new();
/// `--run`: install and start the app once built.
pub static RUN: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Apple's public certificates, sent along with the signature.
const WWDR: &[u8] = include_bytes!("apple/wwdr-g3.cer");
const ROOT: &[u8] = include_bytes!("apple/root.cer");
/// The rules of `CodeResources`, as `codesign` writes them.
const RULES: &str = include_str!("apple/rules.txt");

/// Signs the app folder `dir` (holding `Info.plist` and the executable `stem`).
pub fn bundle(dir: &Path, stem: &str, id: &str) -> Result<(), String> {
    let (Some(profile), Some(identity)) = (PROFILE.get(), IDENTITY.get()) else {
        #[cfg(target_os = "macos")]
        return mac::bundle(dir, id);
        #[cfg(not(target_os = "macos"))]
        return Err(t!("sign_needs"));
    };
    let read = |file: &str| std::fs::read(file).map_err(|e| t!("sign_read", file = file, detail = e));
    let profile_bytes = read(profile)?;
    let identity_text = String::from_utf8_lossy(&read(identity)?).into_owned();
    let profile_info = Profile::read(&profile_bytes).ok_or_else(|| t!("sign_bad", file = profile, what = "provisioning profile"))?;
    if profile_info.expires.as_str() < now_iso().as_str() {
        return Err(t!("sign_expired", date = profile_info.expires));
    }
    if !profile_info.allows(id) {
        return Err(t!("sign_app_id", allowed = profile_info.application, id = id));
    }
    let (key, leaf) = pick_identity(&identity_text, &profile_info).ok_or_else(|| t!("sign_no_cert", file = identity, profile = profile))?;
    let made = |e: std::io::Error| e.to_string();
    let info = std::fs::read(dir.join("Info.plist")).map_err(made)?;
    std::fs::write(dir.join("embedded.mobileprovision"), &profile_bytes).map_err(made)?;
    let resources = resources(&info, &profile_bytes);
    std::fs::create_dir_all(dir.join("_CodeSignature")).map_err(made)?;
    std::fs::write(dir.join("_CodeSignature/CodeResources"), &resources).map_err(made)?;
    let entitlements = profile_info.entitlements_for(id);
    let exe_path = dir.join(stem);
    let exe = std::fs::read(&exe_path).map_err(made)?;
    let signed = sign_executable(&exe, id, &profile_info.team, &entitlements, &info, &resources, &key, &leaf)?;
    std::fs::write(&exe_path, signed).map_err(made)
}

// ---- The executable ----

fn be32(v: &mut Vec<u8>, n: u64) {
    v.extend_from_slice(&(n as u32).to_be_bytes());
}

fn be64(v: &mut Vec<u8>, n: u64) {
    v.extend_from_slice(&n.to_be_bytes());
}

fn le32(b: &[u8], at: usize) -> u64 {
    u32::from_le_bytes(b[at..at + 4].try_into().unwrap()) as u64
}

fn le64(b: &[u8], at: usize) -> u64 {
    u64::from_le_bytes(b[at..at + 8].try_into().unwrap())
}

const PAGE: usize = 4096;

/// A blob: its magic, its length, its content.
fn blob(magic: u32, content: &[u8]) -> Vec<u8> {
    let mut b = Vec::new();
    be32(&mut b, magic as u64);
    be32(&mut b, content.len() as u64 + 8);
    b.extend_from_slice(content);
    b
}

#[allow(clippy::too_many_arguments)]
fn sign_executable(exe: &[u8], id: &str, team: &str, entitlements: &Plist, info: &[u8], resources: &[u8], key: &RsaPrivateKey, leaf: &[u8]) -> Result<Vec<u8>, String> {
    let bad = || "not a Mach-O executable".to_string();
    if exe.len() < 32 || le32(exe, 0) != 0xFEED_FACF {
        return Err(bad());
    }
    // The load commands we change: the signature's, and the segment the signature ends.
    let (mut code, mut linkedit, mut text) = (None, None, (0, 0));
    let mut at = 32;
    for _ in 0..le32(exe, 16) {
        let (cmd, size) = (le32(exe, at), le32(exe, at + 4) as usize);
        match cmd {
            0x1D => code = Some(at),
            0x19 => {
                let name = &exe[at + 8..at + 24];
                if name.starts_with(b"__LINKEDIT\0") {
                    linkedit = Some(at);
                } else if name.starts_with(b"__TEXT\0") {
                    text = (le64(exe, at + 40), le64(exe, at + 48));
                }
            }
            _ => {}
        }
        at += size;
    }
    let (code, linkedit) = (code.ok_or_else(bad)?, linkedit.ok_or_else(bad)?);
    let signed_at = le32(exe, code + 8) as usize;
    let mut file = exe[..signed_at].to_vec();

    let xml = blob(0xFADE_7171, entitlements.xml().as_bytes());
    let der = blob(0xFADE_7172, &entitlements.der());
    let requirements = requirements(id, &common_name(leaf).ok_or_else(bad)?);
    let special = [sha256(info), sha256(&requirements), sha256(resources), [0; 32], sha256(&xml), [0; 32], sha256(&der)];
    // The size of the signature does not depend on the hashes: a first pass tells it, the headers
    // are then written, and the second pass hashes the file with them in.
    let size_of = |hashes: &[[u8; 32]], directory_hash: [u8; 32]| -> Result<Vec<u8>, String> {
        let directory = code_directory(id, team, &special, hashes, signed_at as u64, text);
        let cms = cms(directory_hash, key, leaf)?;
        Ok(assemble(&directory, &requirements, &xml, &der, &cms))
    };
    let pages = signed_at.div_ceil(PAGE);
    let size = size_of(&vec![[0; 32]; pages], [0; 32])?.len() as u64;
    let linkedit_at = le64(exe, linkedit + 40);
    let filesize = signed_at as u64 + size - linkedit_at;
    file[code + 12..code + 16].copy_from_slice(&(size as u32).to_le_bytes());
    file[linkedit + 32..linkedit + 40].copy_from_slice(&filesize.next_multiple_of(0x4000).to_le_bytes());
    file[linkedit + 48..linkedit + 56].copy_from_slice(&filesize.to_le_bytes());
    let hashes: Vec<[u8; 32]> = file.chunks(PAGE).map(sha256).collect();
    let directory = code_directory(id, team, &special, &hashes, signed_at as u64, text);
    let signature = assemble(&directory, &requirements, &xml, &der, &cms(sha256(&directory), key, leaf)?);
    debug_assert_eq!(signature.len() as u64, size);
    file.extend_from_slice(&signature);
    Ok(file)
}

/// The code directory: who the app is, and the hash of everything that makes it.
fn code_directory(id: &str, team: &str, special: &[[u8; 32]; 7], pages: &[[u8; 32]], limit: u64, text: (u64, u64)) -> Vec<u8> {
    let identifier = 88;
    let team_at = identifier + id.len() as u64 + 1;
    let hashes = team_at + team.len() as u64 + 1 + 32 * 7;
    let length = hashes + 32 * pages.len() as u64;
    let mut d = Vec::new();
    // Version 2.4, no flags; the hashes, the name, the special slots, the page count, the code limit.
    for v in [0xFADE_0C02, length, 0x2_0400, 0, hashes, identifier, 7, pages.len() as u64, limit] {
        be32(&mut d, v);
    }
    d.extend_from_slice(&[32, 2, 0, 12]); // SHA-256 of 32 bytes, pages of 2^12
    for v in [0, 0, team_at, 0] {
        be32(&mut d, v); // spare, no scatter, the team, spare
    }
    // The code is `limit` bytes; the executable segment is __TEXT, "main binary" and debuggable.
    for v in [0, text.0, text.1, 1 | 0x10] {
        be64(&mut d, v);
    }
    d.extend_from_slice(id.as_bytes());
    d.push(0);
    d.extend_from_slice(team.as_bytes());
    d.push(0);
    // The slots are written from the 7th down to the 1st, so that the 1st sits just before the pages.
    for slot in special.iter().rev() {
        d.extend_from_slice(slot);
    }
    for page in pages {
        d.extend_from_slice(page);
    }
    d
}

fn assemble(directory: &[u8], requirements: &[u8], xml: &[u8], der: &[u8], cms: &[u8]) -> Vec<u8> {
    let parts: [(u64, &[u8]); 5] = [(0, directory), (2, requirements), (5, xml), (7, der), (0x10000, cms)];
    let mut s = Vec::new();
    be32(&mut s, 0xFADE_0CC0);
    be32(&mut s, 12 + 8 * 5 + parts.iter().map(|p| p.1.len() as u64).sum::<u64>());
    be32(&mut s, 5);
    let mut at = 12 + 8 * 5;
    for (kind, bytes) in parts {
        be32(&mut s, kind);
        be32(&mut s, at);
        at += bytes.len() as u64;
    }
    for (_, bytes) in parts {
        s.extend_from_slice(bytes);
    }
    s
}

/// The requirement an app must meet to be itself: its name, Apple's chain, the developer's certificate.
fn requirements(id: &str, common_name: &str) -> Vec<u8> {
    fn word(v: &mut Vec<u8>, n: u32) {
        v.extend_from_slice(&n.to_be_bytes());
    }
    fn text(v: &mut Vec<u8>, bytes: &[u8]) {
        word(v, bytes.len() as u32);
        v.extend_from_slice(bytes);
        v.resize(v.len().next_multiple_of(4), 0);
    }
    let mut e = Vec::new();
    word(&mut e, 6); // and
    word(&mut e, 2); // identifier
    text(&mut e, id.as_bytes());
    word(&mut e, 6);
    word(&mut e, 15); // anchor apple generic
    word(&mut e, 6);
    word(&mut e, 11); // certificate leaf [subject.CN] = name
    word(&mut e, 0);
    text(&mut e, b"subject.CN");
    word(&mut e, 1);
    text(&mut e, common_name.as_bytes());
    word(&mut e, 14); // certificate 1 [field 1.2.840.113635.100.6.2.1] exists
    word(&mut e, 1);
    text(&mut e, &[0x2A, 0x86, 0x48, 0x86, 0xF7, 0x63, 0x64, 0x06, 0x02, 0x01]);
    word(&mut e, 0);
    let mut one = Vec::new();
    word(&mut one, 1); // the expression's form
    one.extend_from_slice(&e);
    let one = blob(0xFADE_0C00, &one);
    let mut s = Vec::new();
    word(&mut s, 0xFADE_0C01);
    word(&mut s, 12 + 8 + one.len() as u32);
    word(&mut s, 1);
    word(&mut s, 3); // the designated requirement
    word(&mut s, 20);
    s.extend_from_slice(&one);
    s
}

// ---- The signature (CMS, RFC 5652) ----

fn tlv(tag: u8, content: &[u8]) -> Vec<u8> {
    let mut v = vec![tag];
    match content.len() {
        n if n < 128 => v.push(n as u8),
        n if n < 256 => v.extend_from_slice(&[0x81, n as u8]),
        n => v.extend_from_slice(&[0x82, (n >> 8) as u8, n as u8]),
    }
    v.extend_from_slice(content);
    v
}

fn cat(parts: &[&[u8]]) -> Vec<u8> {
    parts.concat()
}

fn oid(bytes: &[u8]) -> Vec<u8> {
    tlv(6, bytes)
}

const SHA256_OID: [u8; 9] = [0x60, 0x86, 0x48, 0x01, 0x65, 0x03, 0x04, 0x02, 0x01];
const DATA_OID: [u8; 9] = [0x2A, 0x86, 0x48, 0x86, 0xF7, 0x0D, 0x01, 0x07, 0x01];

fn attribute(kind: &[u8], value: &[u8]) -> Vec<u8> {
    tlv(0x30, &cat(&[&oid(kind), &tlv(0x31, value)]))
}

/// The detached signature of `directory` by `key`, with the chain that vouches for `leaf`.
fn cms(hash: [u8; 32], key: &RsaPrivateKey, leaf: &[u8]) -> Result<Vec<u8>, String> {
    let parts = Certificate::read(leaf).ok_or("unreadable certificate")?;
    let algorithm = tlv(0x30, &cat(&[&oid(&SHA256_OID), &[5, 0]]));
    let plist = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n<plist version=\"1.0\">\n<dict>\n\t<key>cdhashes</key>\n\t<array>\n\t\t<data>\n\t\t{}\n\t\t</data>\n\t</array>\n</dict>\n</plist>\n",
        base64(&hash[..20], false)
    );
    let time = now_iso();
    let utc = format!("{}{}{}{}{}{}Z", &time[2..4], &time[5..7], &time[8..10], &time[11..13], &time[14..16], &time[17..19]);
    let attributes = cat(&[
        &attribute(&[0x2A, 0x86, 0x48, 0x86, 0xF7, 0x0D, 0x01, 0x09, 0x03], &oid(&DATA_OID)),
        &attribute(&[0x2A, 0x86, 0x48, 0x86, 0xF7, 0x0D, 0x01, 0x09, 0x05], &tlv(0x17, utc.as_bytes())),
        &attribute(&[0x2A, 0x86, 0x48, 0x86, 0xF7, 0x0D, 0x01, 0x09, 0x04], &tlv(4, &hash)),
        &attribute(&[0x2A, 0x86, 0x48, 0x86, 0xF7, 0x63, 0x64, 0x09, 0x02], &tlv(0x30, &cat(&[&oid(&SHA256_OID), &tlv(4, &hash)]))),
        &attribute(&[0x2A, 0x86, 0x48, 0x86, 0xF7, 0x63, 0x64, 0x09, 0x01], &tlv(4, plist.as_bytes())),
    ]);
    // What is signed is the attributes as a SET; what is sent is the same bytes tagged [0].
    let signing_key = rsa::pkcs1v15::SigningKey::<rsa::sha2::Sha256>::new(key.clone());
    let signature = signing_key.sign(&tlv(0x31, &attributes)).to_bytes();
    let rsa_sha256 = tlv(0x30, &cat(&[&oid(&[0x2A, 0x86, 0x48, 0x86, 0xF7, 0x0D, 0x01, 0x01, 0x0B]), &[5, 0]]));
    let signer = tlv(
        0x30,
        &cat(&[&[2, 1, 1], &tlv(0x30, &cat(&[&parts.issuer, &parts.serial])), &algorithm, &tlv(0xA0, &attributes), &rsa_sha256, &tlv(4, &signature)]),
    );
    let signed = tlv(
        0x30,
        &cat(&[&[2, 1, 1], &tlv(0x31, &algorithm), &tlv(0x30, &oid(&DATA_OID)), &tlv(0xA0, &cat(&[WWDR, ROOT, leaf])), &tlv(0x31, &signer)]),
    );
    let content = tlv(0x30, &cat(&[&oid(&[0x2A, 0x86, 0x48, 0x86, 0xF7, 0x0D, 0x01, 0x07, 0x02]), &tlv(0xA0, &signed)]));
    Ok(blob(0xFADE_0B01, &content))
}

/// The parts of a certificate the signature needs.
struct Certificate {
    issuer: Vec<u8>,
    serial: Vec<u8>,
    subject: Vec<u8>,
    public_key: Vec<u8>,
}

/// One DER element: its tag, its whole bytes and its content.
fn der(b: &[u8]) -> Option<(u8, &[u8], &[u8], &[u8])> {
    let tag = *b.first()?;
    let (len, head): (usize, usize) = match *b.get(1)? {
        n if n < 128 => (n as usize, 2),
        0x81 => (*b.get(2)? as usize, 3),
        0x82 => ((*b.get(2)? as usize) << 8 | *b.get(3)? as usize, 4),
        0x83 => ((*b.get(2)? as usize) << 16 | (*b.get(3)? as usize) << 8 | *b.get(4)? as usize, 5),
        _ => return None,
    };
    let end = len + head as usize;
    Some((tag, b.get(..end)?, b.get(head..end)?, &b[end..]))
}

impl Certificate {
    fn read(cert: &[u8]) -> Option<Certificate> {
        let (_, _, body, _) = der(cert)?;
        let (_, _, tbs, _) = der(body)?;
        let mut rest = tbs;
        let (mut tag, mut whole, _, mut next) = der(rest)?;
        if tag == 0xA0 {
            rest = next;
            (tag, whole, _, next) = der(rest)?;
        }
        let _ = tag;
        let serial = whole.to_vec();
        let (_, _, _, rest) = der(next)?; // the algorithm
        let (_, issuer, _, rest) = der(rest)?;
        let (_, _, _, rest) = der(rest)?; // the dates
        let (_, subject, _, rest) = der(rest)?;
        let (_, key, _, _) = der(rest)?;
        Some(Certificate { issuer: issuer.to_vec(), serial, subject: subject.to_vec(), public_key: key.to_vec() })
    }
}

/// The common name ("Apple Development: …") of a certificate's subject.
fn common_name(cert: &[u8]) -> Option<String> {
    subject_field(cert, 3)
}

/// A field of a certificate's subject: 3 is the common name, 11 the organizational unit (the team).
fn subject_field(cert: &[u8], field: u8) -> Option<String> {
    let subject = Certificate::read(cert)?.subject;
    let (_, _, mut names, _) = der(&subject)?;
    while !names.is_empty() {
        let (_, _, set, next) = der(names)?;
        let (_, _, pair, _) = der(set)?;
        let (_, _, kind, value) = der(pair)?;
        if kind == [0x55, 0x04, field] {
            return Some(String::from_utf8_lossy(der(value)?.2).into_owned());
        }
        names = next;
    }
    None
}

/// The private key and the certificate (DER) that belong together among those of the identity
/// file and whose certificate the profile lists.
fn pick_identity(text: &str, profile: &Profile) -> Option<(RsaPrivateKey, Vec<u8>)> {
    let blocks = pem_blocks(text);
    let keys: Vec<RsaPrivateKey> = blocks
        .iter()
        .filter(|(kind, _)| kind.ends_with("PRIVATE KEY"))
        .filter_map(|(kind, body)| {
            let pem = format!("-----BEGIN {kind}-----\n{body}\n-----END {kind}-----\n");
            if kind == "RSA PRIVATE KEY" {
                rsa::pkcs1::DecodeRsaPrivateKey::from_pkcs1_pem(&pem).ok()
            } else {
                RsaPrivateKey::from_pkcs8_pem(&pem).ok()
            }
        })
        .collect();
    for (kind, body) in &blocks {
        if kind != "CERTIFICATE" {
            continue;
        }
        let Some(cert) = from_base64(body, false) else { continue };
        if !profile.certificates.contains(&cert) {
            continue;
        }
        let Some(parts) = Certificate::read(&cert) else { continue };
        let Ok(public) = rsa::RsaPublicKey::from_public_key_der(&parts.public_key) else { continue };
        if let Some(key) = keys.iter().find(|k| k.n() == public.n()) {
            return Some((key.clone(), cert));
        }
    }
    None
}

/// `(kind, base64 body)` of each `-----BEGIN kind-----` block.
fn pem_blocks(text: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut rest = text;
    while let Some(start) = rest.find("-----BEGIN ") {
        let after = &rest[start + 11..];
        let Some(end) = after.find("-----") else { break };
        let kind = after[..end].to_string();
        let body_start = &after[end + 5..];
        let Some(stop) = body_start.find("-----END ") else { break };
        out.push((kind, body_start[..stop].split_whitespace().collect::<Vec<_>>().join("\n")));
        rest = &body_start[stop + 9..];
    }
    out
}

// ---- The documents around the executable ----

/// `_CodeSignature/CodeResources`: the hashes of the files that are not the executable.
fn resources(info: &[u8], profile: &[u8]) -> Vec<u8> {
    let data = |bytes: &[u8]| format!("<data>\n\t\t{}\n\t\t</data>", base64(bytes, false));
    let mut s = String::from("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n<plist version=\"1.0\">\n<dict>\n\t<key>files</key>\n\t<dict>\n");
    s += &format!("\t\t<key>Info.plist</key>\n\t\t{}\n", data(&sha1(info)));
    s += &format!("\t\t<key>embedded.mobileprovision</key>\n\t\t{}\n", data(&sha1(profile)));
    s += "\t</dict>\n\t<key>files2</key>\n\t<dict>\n\t\t<key>embedded.mobileprovision</key>\n\t\t<dict>\n\t\t\t<key>hash2</key>\n";
    s += &format!("\t\t\t{}\n", data(&sha256(profile)).replace("\n\t\t", "\n\t\t\t").replace("</data>", "</data>"));
    s += "\t\t</dict>\n\t</dict>\n";
    s += RULES;
    s.into_bytes()
}

fn sha1(data: &[u8]) -> [u8; 20] {
    let mut h: [u32; 5] = [0x6745_2301, 0xEFCD_AB89, 0x98BA_DCFE, 0x1032_5476, 0xC3D2_E1F0];
    let mut message = data.to_vec();
    message.push(0x80);
    while message.len() % 64 != 56 {
        message.push(0);
    }
    message.extend_from_slice(&(data.len() as u64 * 8).to_be_bytes());
    for block in message.chunks(64) {
        let mut w = [0u32; 80];
        for i in 0..16 {
            w[i] = u32::from_be_bytes(block[i * 4..i * 4 + 4].try_into().unwrap());
        }
        for i in 16..80 {
            w[i] = (w[i - 3] ^ w[i - 8] ^ w[i - 14] ^ w[i - 16]).rotate_left(1);
        }
        let [mut a, mut b, mut c, mut d, mut e] = h;
        for (i, &word) in w.iter().enumerate() {
            let (f, k) = match i {
                0..=19 => (b & c | !b & d, 0x5A82_7999),
                20..=39 => (b ^ c ^ d, 0x6ED9_EBA1),
                40..=59 => (b & c | b & d | c & d, 0x8F1B_BCDC),
                _ => (b ^ c ^ d, 0xCA62_C1D6u32),
            };
            let t = a.rotate_left(5).wrapping_add(f).wrapping_add(e).wrapping_add(k).wrapping_add(word);
            (e, d, c, b, a) = (d, c, b.rotate_left(30), a, t);
        }
        for (x, y) in h.iter_mut().zip([a, b, c, d, e]) {
            *x = x.wrapping_add(y);
        }
    }
    let mut out = [0u8; 20];
    for (i, x) in h.iter().enumerate() {
        out[i * 4..i * 4 + 4].copy_from_slice(&x.to_be_bytes());
    }
    out
}

// ---- Property lists ----

/// What a property list holds (enough for a provisioning profile and entitlements).
#[derive(Clone, Debug, PartialEq)]
pub enum Plist {
    Dict(Vec<(String, Plist)>),
    Array(Vec<Plist>),
    Text(String),
    Bool(bool),
    Number(String),
    Data(Vec<u8>),
    Date(String),
}

impl Plist {
    /// The value written under `key` of a dictionary.
    fn get(&self, key: &str) -> Option<&Plist> {
        match self {
            Plist::Dict(pairs) => pairs.iter().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }

    fn text(&self) -> Option<&str> {
        match self {
            Plist::Text(t) | Plist::Date(t) => Some(t),
            _ => None,
        }
    }

    pub fn parse(text: &str) -> Option<Plist> {
        let start = text.find("<plist")?;
        let mut at = start + text[start..].find('>')? + 1;
        value(text, &mut at)
    }

    /// The XML a signature carries, laid out as `codesign` does.
    pub fn xml(&self) -> String {
        let mut s = String::from("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n<plist version=\"1.0\">\n");
        self.write(&mut s, 0);
        s + "</plist>\n"
    }

    fn write(&self, s: &mut String, depth: usize) {
        let tabs = "\t".repeat(depth);
        let escaped = |t: &str| t.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;");
        match self {
            Plist::Dict(pairs) if pairs.is_empty() => *s += &format!("{tabs}<dict/>\n"),
            Plist::Dict(pairs) => {
                *s += &format!("{tabs}<dict>\n");
                for (k, v) in pairs {
                    *s += &format!("{tabs}\t<key>{}</key>\n", escaped(k));
                    v.write(s, depth + 1);
                }
                *s += &format!("{tabs}</dict>\n");
            }
            Plist::Array(items) if items.is_empty() => *s += &format!("{tabs}<array/>\n"),
            Plist::Array(items) => {
                *s += &format!("{tabs}<array>\n");
                for v in items {
                    v.write(s, depth + 1);
                }
                *s += &format!("{tabs}</array>\n");
            }
            Plist::Text(t) => *s += &format!("{tabs}<string>{}</string>\n", escaped(t)),
            Plist::Bool(b) => *s += &format!("{tabs}<{b}/>\n"),
            Plist::Number(n) => *s += &format!("{tabs}<integer>{n}</integer>\n"),
            Plist::Data(d) => *s += &format!("{tabs}<data>\n{tabs}{}\n{tabs}</data>\n", base64(d, false)),
            Plist::Date(d) => *s += &format!("{tabs}<date>{d}</date>\n"),
        }
    }

    /// The DER form the system reads entitlements in (a tagged set of key / value pairs).
    pub fn der(&self) -> Vec<u8> {
        tlv(0x70, &cat(&[&[2, 1, 1], &self.der_value()]))
    }

    fn der_value(&self) -> Vec<u8> {
        match self {
            Plist::Dict(pairs) => {
                let mut body = Vec::new();
                for (k, v) in pairs {
                    body.extend(tlv(0x30, &cat(&[&tlv(0x0C, k.as_bytes()), &v.der_value()])));
                }
                tlv(0xB0, &body)
            }
            Plist::Array(items) => tlv(0x30, &items.iter().flat_map(|v| v.der_value()).collect::<Vec<u8>>()),
            Plist::Text(t) | Plist::Date(t) => tlv(0x0C, t.as_bytes()),
            Plist::Bool(b) => tlv(1, &[if *b { 0xFF } else { 0 }]),
            Plist::Number(n) => {
                let n: i64 = n.parse().unwrap_or(0);
                let bytes = n.to_be_bytes();
                let skip = bytes.iter().take(7).zip(bytes.iter().skip(1)).take_while(|(a, b)| (**a == 0 && **b & 0x80 == 0) || (**a == 0xFF && **b & 0x80 != 0)).count();
                tlv(2, &bytes[skip..])
            }
            Plist::Data(d) => tlv(4, d),
        }
    }
}

fn skip(text: &str, at: &mut usize) {
    while text[*at..].starts_with(|c: char| c.is_whitespace()) {
        *at += text[*at..].chars().next().unwrap().len_utf8();
    }
}

fn unescaped(t: &str) -> String {
    t.replace("&lt;", "<").replace("&gt;", ">").replace("&quot;", "\"").replace("&apos;", "'").replace("&amp;", "&")
}

fn value(text: &str, at: &mut usize) -> Option<Plist> {
    skip(text, at);
    let rest = &text[*at..];
    if !rest.starts_with('<') {
        return None;
    }
    let close = rest.find('>')?;
    let tag = &rest[1..close];
    *at += close + 1;
    let empty = tag.ends_with('/');
    let name = tag.trim_end_matches('/').trim();
    match (name, empty) {
        ("true", _) => Some(Plist::Bool(true)),
        ("false", _) => Some(Plist::Bool(false)),
        ("dict", true) => Some(Plist::Dict(vec![])),
        ("array", true) => Some(Plist::Array(vec![])),
        ("string" | "data" | "date" | "integer" | "real", true) => Some(Plist::Text(String::new())),
        ("dict", false) => {
            let mut pairs = Vec::new();
            loop {
                skip(text, at);
                if text[*at..].starts_with("</dict>") {
                    *at += 7;
                    return Some(Plist::Dict(pairs));
                }
                let Plist::Text(key) = value(text, at)? else { return None };
                pairs.push((key, value(text, at)?));
            }
        }
        ("array", false) => {
            let mut items = Vec::new();
            loop {
                skip(text, at);
                if text[*at..].starts_with("</array>") {
                    *at += 8;
                    return Some(Plist::Array(items));
                }
                items.push(value(text, at)?);
            }
        }
        ("key" | "string" | "data" | "date" | "integer" | "real", false) => {
            let end = format!("</{name}>");
            let stop = text[*at..].find(&end)?;
            let body = unescaped(&text[*at..*at + stop]);
            *at += stop + end.len();
            Some(match name {
                "key" | "string" => Plist::Text(body),
                "data" => Plist::Data(from_base64(&body, false)?),
                "date" => Plist::Date(body),
                _ => Plist::Number(body),
            })
        }
        _ => None,
    }
}

// ---- The provisioning profile ----

/// What Kova reads of a `.mobileprovision` file.
struct Profile {
    team: String,
    /// `TEAM.dev.kova.essai`, or `TEAM.*`.
    application: String,
    expires: String,
    certificates: Vec<Vec<u8>>,
    entitlements: Plist,
}

impl Profile {
    fn read(bytes: &[u8]) -> Option<Profile> {
        // The profile is a signed envelope whose content, a property list, is in clear.
        let text = String::from_utf8_lossy(bytes);
        let start = text.find("<?xml")?;
        let end = text.find("</plist>")? + 8;
        let plist = Plist::parse(&text[start..end])?;
        let entitlements = plist.get("Entitlements")?.clone();
        let application = entitlements.get("application-identifier")?.text()?.to_string();
        let team = match plist.get("TeamIdentifier")? {
            Plist::Array(items) => items.first()?.text()?.to_string(),
            _ => return None,
        };
        let certificates = match plist.get("DeveloperCertificates")? {
            Plist::Array(items) => items.iter().filter_map(|i| if let Plist::Data(d) = i { Some(d.clone()) } else { None }).collect(),
            _ => return None,
        };
        Some(Profile { team, application, expires: plist.get("ExpirationDate")?.text()?.to_string(), certificates, entitlements })
    }

    /// Whether the app `id` (`dev.kova.essai`) may be signed with this profile.
    fn allows(&self, id: &str) -> bool {
        let Some(allowed) = self.application.strip_prefix(&format!("{}.", self.team)) else { return false };
        allowed == id || allowed.strip_suffix('*').is_some_and(|prefix| id.starts_with(prefix))
    }

    /// The profile's entitlements, with the full name of the app (a wildcard profile says `TEAM.*`).
    fn entitlements_for(&self, id: &str) -> Plist {
        let Plist::Dict(pairs) = &self.entitlements else { return self.entitlements.clone() };
        let mut pairs: Vec<(String, Plist)> = pairs.iter().map(|(k, v)| if k == "application-identifier" { (k.clone(), Plist::Text(format!("{}.{id}", self.team))) } else { (k.clone(), v.clone()) }).collect();
        pairs.sort_by(|a, b| a.0.cmp(&b.0));
        Plist::Dict(pairs)
    }
}

// ---- The clock ----

/// Now, as `2026-11-08T12:00:00Z`.
fn now_iso() -> String {
    let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs()) as i64;
    let (days, rest) = (secs.div_euclid(86400), secs.rem_euclid(86400));
    // Civil date from days since 1970 (Howard Hinnant's algorithm).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + (month <= 2) as i64;
    format!("{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z", rest / 3600, rest % 3600 / 60, rest % 60)
}

// ---- On a Mac: Xcode's own files and `codesign` ----

/// What a Mac already has: Xcode's profiles and the keychain. Nothing to give to Kova.
#[cfg(target_os = "macos")]
mod mac {
    use super::*;
    use std::process::Command;

    fn profile_folders() -> Vec<std::path::PathBuf> {
        let home = std::env::var("HOME").unwrap_or_default();
        ["Library/Developer/Xcode/UserData/Provisioning Profiles", "Library/MobileDevice/Provisioning Profiles"].iter().map(|f| Path::new(&home).join(f)).collect()
    }

    /// Every profile Xcode keeps, read.
    fn profiles() -> Vec<(Vec<u8>, Profile)> {
        let mut all = Vec::new();
        for folder in profile_folders() {
            for entry in std::fs::read_dir(folder).into_iter().flatten().flatten() {
                if entry.path().extension().is_some_and(|e| e == "mobileprovision") {
                    if let Some(bytes) = std::fs::read(entry.path()).ok().filter(|b| Profile::read(b).is_some()) {
                        let profile = Profile::read(&bytes).unwrap();
                        all.push((bytes, profile));
                    }
                }
            }
        }
        all
    }

    /// The profile of the app `id`: one that exists and is valid, an exact one before a wildcard
    /// one, else the one Xcode makes for it.
    fn profile_for(id: &str) -> Result<(Vec<u8>, Profile), String> {
        let find = || {
            let now = now_iso();
            let mut usable: Vec<_> = profiles().into_iter().filter(|(_, p)| p.allows(id) && p.expires > now).collect();
            usable.sort_by_key(|(_, p)| p.application.ends_with('*'));
            usable.into_iter().next()
        };
        if let Some(found) = find() {
            return Ok(found);
        }
        let team = profiles().first().map(|(_, p)| p.team.clone()).or_else(team_from_keychain).ok_or_else(|| t!("sign_no_team"))?;
        make_profile(id, &team)?;
        find().ok_or_else(|| t!("sign_no_profile", id = id))
    }

    fn team_from_keychain() -> Option<String> {
        let out = Command::new("security").args(["find-certificate", "-c", "Apple Development", "-p"]).output().ok()?;
        let text = String::from_utf8_lossy(&out.stdout).into_owned();
        let (_, body) = pem_blocks(&text).into_iter().find(|(k, _)| k == "CERTIFICATE")?;
        subject_field(&from_base64(&body, false)?, 11)
    }

    /// Asks Xcode for the profile of `id` the way it does for its own projects: a tiny project is
    /// "built" with `-allowProvisioningUpdates`, which registers the name and the iPhone with Apple
    /// and downloads the profile. Whether the build then succeeds does not matter.
    fn make_profile(id: &str, team: &str) -> Result<(), String> {
        let dir = std::env::temp_dir().join(format!("kova-profile-{}", std::process::id()));
        let made = |e: std::io::Error| e.to_string();
        std::fs::create_dir_all(dir.join("K.xcodeproj")).map_err(made)?;
        std::fs::write(dir.join("main.c"), "int main(void) { return 0; }\n").map_err(made)?;
        let project = XCODE_PROJECT.replace("@ID@", id).replace("@TEAM@", team);
        std::fs::write(dir.join("K.xcodeproj/project.pbxproj"), project).map_err(made)?;
        eprintln!("{}", t!("sign_asking", id = id));
        let run = Command::new("xcodebuild")
            .current_dir(&dir)
            .args(["-project", "K.xcodeproj", "-target", "K", "-sdk", "iphoneos", "-configuration", "Release", "-allowProvisioningUpdates", "-allowProvisioningDeviceRegistration", "build"])
            .output()
            .map_err(|e| t!("sign_tool", tool = "xcodebuild", detail = e))?;
        let _ = std::fs::remove_dir_all(&dir);
        let text = String::from_utf8_lossy(&run.stdout).into_owned() + &String::from_utf8_lossy(&run.stderr);
        if let Some(line) = text.lines().rev().find(|l| l.contains("error:") && (l.contains("Apple ID") || l.contains("account") || l.contains("Provisioning") || l.contains("team"))) {
            return Err(t!("sign_xcode", detail = line.trim()));
        }
        Ok(())
    }

    /// Signs `dir` with `codesign` and the certificate of the profile that is in the keychain.
    pub fn bundle(dir: &Path, id: &str) -> Result<(), String> {
        let (bytes, profile) = profile_for(id)?;
        let made = |e: std::io::Error| e.to_string();
        std::fs::write(dir.join("embedded.mobileprovision"), &bytes).map_err(made)?;
        let listed = Command::new("security").args(["find-identity", "-v", "-p", "codesigning"]).output().map_err(|e| t!("sign_tool", tool = "security", detail = e))?;
        let listed = String::from_utf8_lossy(&listed.stdout).into_owned();
        let hex = |d: &[u8; 20]| d.iter().map(|b| format!("{b:02X}")).collect::<String>();
        let wanted: Vec<String> = profile.certificates.iter().map(|c| hex(&sha1(c))).collect();
        let identity = listed.lines().filter_map(|l| l.split_whitespace().nth(1)).find(|h| wanted.iter().any(|w| w == h)).ok_or_else(|| t!("sign_no_identity"))?;
        let entitlements = std::env::temp_dir().join(format!("kova-entitlements-{}.plist", std::process::id()));
        std::fs::write(&entitlements, profile.entitlements_for(id).xml()).map_err(made)?;
        let run = Command::new("codesign").args(["-f", "-s", identity, "--entitlements"]).arg(&entitlements).arg(dir).output().map_err(|e| t!("sign_tool", tool = "codesign", detail = e));
        let _ = std::fs::remove_file(&entitlements);
        let run = run?;
        if run.status.success() {
            Ok(())
        } else {
            Err(t!("sign_codesign", detail = String::from_utf8_lossy(&run.stderr).trim()))
        }
    }

    const XCODE_PROJECT: &str = include_str!("apple/project.pbxproj");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sha1_of_abc() {
        assert_eq!(sha1(b"abc").iter().map(|b| format!("{b:02x}")).collect::<String>(), "a9993e364706816aba3e25717850c26c9cd0d89d");
    }

    /// The entitlements of an app signed by `codesign`, in XML and in DER.
    #[test]
    fn entitlements_as_codesign_writes_them() {
        let xml = "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n<plist version=\"1.0\">\n<dict>\n\t<key>application-identifier</key>\n\t<string>J2XVNQPWH7.dev.kova.essai</string>\n\t<key>com.apple.developer.team-identifier</key>\n\t<string>J2XVNQPWH7</string>\n\t<key>get-task-allow</key>\n\t<true/>\n\t<key>keychain-access-groups</key>\n\t<array>\n\t\t<string>J2XVNQPWH7.*</string>\n\t</array>\n</dict>\n</plist>\n";
        let plist = Plist::parse(xml).unwrap();
        assert_eq!(plist.xml(), xml);
        let der = "7081ad020101b081a730330c166170706c69636174696f6e2d6964656e7469666965720c194a3258564e51505748372e6465762e6b6f76612e657373616930310c23636f6d2e6170706c652e646576656c6f7065722e7465616d2d6964656e7469666965720c0a4a3258564e515057483730130c0e6765742d7461736b2d616c6c6f770101ff30280c166b6579636861696e2d6163636573732d67726f757073300e0c0c4a3258564e51505748372e2a";
        assert_eq!(plist.der().iter().map(|b| format!("{b:02x}")).collect::<String>(), der);
    }

    #[test]
    fn requirement_of_an_app() {
        let blob = requirements("dev.kova.essai", "Apple Development: a@b.c (ABCDE12345)");
        assert_eq!(&blob[..4], &[0xFA, 0xDE, 0x0C, 0x01]);
        assert_eq!(u32::from_be_bytes(blob[4..8].try_into().unwrap()) as usize, blob.len());
        assert_eq!(blob.len() % 4, 0);
    }
}

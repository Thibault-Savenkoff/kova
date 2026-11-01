#!/bin/sh
# Makes classes.dex from Main.java. Only needed after changing Main.java: the result is kept in the
# repository, so that nobody building or using Kova needs Android's tools.
# Needs the Android SDK: ANDROID_HOME, build-tools 35, platform 35, and a JDK.
set -e
cd "$(dirname "$0")"
B="$ANDROID_HOME/build-tools/35.0.0"
J="$ANDROID_HOME/platforms/android-35/android.jar"
T=$(mktemp -d)
javac -nowarn -Xlint:none -source 8 -target 8 -cp "$J" -d "$T" Main.java 2>&1 | grep -v '^Note\|warning: \[options\]\|^[0-9]* warning' || true
"$B/d8" --release --min-api 26 --lib "$J" --output "$T" "$T"/dev/kova/*.class
cp "$T/classes.dex" classes.dex
command rm -rf "$T"
ls -la classes.dex

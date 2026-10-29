#!/bin/sh
# Run by the app target after its frameworks are embedded. Two App Store fixes:
#
# ITMS-90208. Xcode links ONNX Runtime's package (a static library built for iOS 15.1) into the app as a dynamic
# framework built for the app's own minimum, but keeps the vendor Info.plist, which says MinimumOSVersion 15.1 and
# names no platform. Each embedded framework's Info.plist gets the minimum its binary was actually built for and its
# platform.
#
# ITMS-91053. The ONNX Runtime 1.24.2 xcframework references stat and fstat (file-timestamp APIs) and ships no
# PrivacyInfo.xcprivacy. Apple scans that framework on its own, so Privacy/OnnxRuntime/PrivacyInfo.xcprivacy is
# copied to the framework root when the vendor did not include one. A vendor manifest is left in place.
#
# A framework that was changed is signed again. Frameworks already right are left alone.
set -eu

dir="$TARGET_BUILD_DIR/$FRAMEWORKS_FOLDER_PATH"
[ -d "$dir" ] || exit 0
case "$PLATFORM_NAME" in
iphoneos) platform=iPhoneOS ;;
iphonesimulator) platform=iPhoneSimulator ;;
*) exit 0 ;;
esac

onnx_privacy="$SRCROOT/Privacy/OnnxRuntime/PrivacyInfo.xcprivacy"

for fw in "$dir"/*.framework; do
    [ -f "$fw/Info.plist" ] || continue
    changed=0
    if [ "$(basename "$fw")" = "onnxruntime.framework" ] && [ ! -f "$fw/PrivacyInfo.xcprivacy" ]; then
        [ -f "$onnx_privacy" ] || {
            echo "error: missing $onnx_privacy" >&2
            exit 1
        }
        cp "$onnx_privacy" "$fw/PrivacyInfo.xcprivacy"
        echo "onnxruntime.framework: added PrivacyInfo.xcprivacy"
        changed=1
    fi

    plist="$fw/Info.plist"
    exe="$(/usr/libexec/PlistBuddy -c "Print :CFBundleExecutable" "$plist")"
    minos="$(xcrun vtool -show-build "$fw/$exe" 2>/dev/null | awk '$1 == "minos" { print $2; exit }')"
    minos="${minos:-$IPHONEOS_DEPLOYMENT_TARGET}"
    current="$(/usr/libexec/PlistBuddy -c "Print :MinimumOSVersion" "$plist" 2>/dev/null || true)"
    platforms="$(/usr/libexec/PlistBuddy -c "Print :CFBundleSupportedPlatforms:0" "$plist" 2>/dev/null || true)"
    if [ "$current" != "$minos" ] || [ "$platforms" != "$platform" ]; then
        echo "$(basename "$fw"): MinimumOSVersion ${current:-none} -> $minos, CFBundleSupportedPlatforms -> $platform"
        /usr/libexec/PlistBuddy -c "Delete :MinimumOSVersion" "$plist" 2>/dev/null || true
        /usr/libexec/PlistBuddy -c "Add :MinimumOSVersion string $minos" "$plist"
        /usr/libexec/PlistBuddy -c "Delete :CFBundleSupportedPlatforms" "$plist" 2>/dev/null || true
        /usr/libexec/PlistBuddy -c "Add :CFBundleSupportedPlatforms array" -c "Add :CFBundleSupportedPlatforms:0 string $platform" "$plist"
        changed=1
    fi
    if [ "$changed" = 1 ] && [ "${CODE_SIGNING_ALLOWED:-YES}" = YES ] && [ -n "${EXPANDED_CODE_SIGN_IDENTITY:-}" ]; then
        codesign --force --sign "$EXPANDED_CODE_SIGN_IDENTITY" --preserve-metadata=identifier,flags --timestamp=none "$fw"
    fi
done

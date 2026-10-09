#!/bin/sh
# Run by the app target after its frameworks are embedded. Xcode links ONNX Runtime's package (a static library built
# for iOS 15.1) into the app as a dynamic framework built for the app's own minimum, but keeps the vendor Info.plist,
# which says MinimumOSVersion 15.1 and names no platform; App Store Connect refuses that (ITMS-90208, "does not support
# the minimum OS Version specified in the Info.plist"). So each embedded framework's Info.plist gets the minimum its
# binary was actually built for and its platform, and the framework is signed again. Frameworks already right are left
# alone.
set -eu

dir="$TARGET_BUILD_DIR/$FRAMEWORKS_FOLDER_PATH"
[ -d "$dir" ] || exit 0
case "$PLATFORM_NAME" in
iphoneos) platform=iPhoneOS ;;
iphonesimulator) platform=iPhoneSimulator ;;
*) exit 0 ;;
esac

for fw in "$dir"/*.framework; do
    [ -f "$fw/Info.plist" ] || continue
    plist="$fw/Info.plist"
    exe="$(/usr/libexec/PlistBuddy -c "Print :CFBundleExecutable" "$plist")"
    minos="$(xcrun vtool -show-build "$fw/$exe" 2>/dev/null | awk '$1 == "minos" { print $2; exit }')"
    minos="${minos:-$IPHONEOS_DEPLOYMENT_TARGET}"
    current="$(/usr/libexec/PlistBuddy -c "Print :MinimumOSVersion" "$plist" 2>/dev/null || true)"
    platforms="$(/usr/libexec/PlistBuddy -c "Print :CFBundleSupportedPlatforms:0" "$plist" 2>/dev/null || true)"
    [ "$current" = "$minos" ] && [ "$platforms" = "$platform" ] && continue

    echo "$(basename "$fw"): MinimumOSVersion ${current:-none} -> $minos, CFBundleSupportedPlatforms -> $platform"
    /usr/libexec/PlistBuddy -c "Delete :MinimumOSVersion" "$plist" 2>/dev/null || true
    /usr/libexec/PlistBuddy -c "Add :MinimumOSVersion string $minos" "$plist"
    /usr/libexec/PlistBuddy -c "Delete :CFBundleSupportedPlatforms" "$plist" 2>/dev/null || true
    /usr/libexec/PlistBuddy -c "Add :CFBundleSupportedPlatforms array" -c "Add :CFBundleSupportedPlatforms:0 string $platform" "$plist"
    if [ "${CODE_SIGNING_ALLOWED:-YES}" = YES ] && [ -n "${EXPANDED_CODE_SIGN_IDENTITY:-}" ]; then
        codesign --force --sign "$EXPANDED_CODE_SIGN_IDENTITY" --preserve-metadata=identifier,flags --timestamp=none "$fw"
    fi
done

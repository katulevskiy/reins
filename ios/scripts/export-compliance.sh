#!/bin/sh
# Run by the app target once its Info.plist is processed. Reins uses standard, non-exempt encryption and is available
# in France, so ios/Reins/Info.plist always says ITSAppUsesNonExemptEncryption = YES, and an App Store build must also
# carry ITSEncryptionExportComplianceCode: the code App Store Connect gives once Apple approves the app's encryption
# declaration (the French one included). With both keys App Store Connect asks nothing per build.
#
# The code is the build setting REINS_EXPORT_COMPLIANCE_CODE (ios/Config/Reins.xcconfig leaves it empty; the release
# workflow sets it from the secret ITS_ENCRYPTION_EXPORT_COMPLIANCE_CODE; Local.xcconfig or -xcconfig can too). The key
# is added only when the code is set, never as an empty string. Without it:
#   - an archive (ACTION=install) fails while REINS_EXPORT_COMPLIANCE_REQUIRED is YES (Release, in Reins.xcconfig), so
#     no App Store or TestFlight build goes out without the code;
#   - any other build (Debug, simulator, tests, a Release build that is not an archive) just leaves the key out.
set -eu

plist="$TARGET_BUILD_DIR/$INFOPLIST_PATH"
[ -f "$plist" ] || { echo "error: export compliance: no Info.plist at $plist"; exit 1; }
code="${REINS_EXPORT_COMPLIANCE_CODE:-}"

uses="$(/usr/libexec/PlistBuddy -c "Print :ITSAppUsesNonExemptEncryption" "$plist" 2>/dev/null || true)"
if [ "$uses" != true ]; then
    echo "error: export compliance: ios/Reins/Info.plist must set ITSAppUsesNonExemptEncryption to YES (it is '${uses:-missing}'): the app uses non-exempt encryption"
    exit 1
fi

# An incremental build may keep the previous build's key: start from none.
/usr/libexec/PlistBuddy -c "Delete :ITSEncryptionExportComplianceCode" "$plist" >/dev/null 2>&1 || true

if [ -n "$code" ]; then
    case "$code" in
    *[!A-Za-z0-9-]*)
        echo "error: export compliance: REINS_EXPORT_COMPLIANCE_CODE must be App Store Connect's code (letters, digits and hyphens)"
        exit 1
        ;;
    esac
    /usr/libexec/PlistBuddy -c "Add :ITSEncryptionExportComplianceCode string $code" "$plist"
    echo "export compliance: ITSAppUsesNonExemptEncryption YES, ITSEncryptionExportComplianceCode set"
elif [ "${REINS_EXPORT_COMPLIANCE_REQUIRED:-NO}" = YES ] && [ "${ACTION:-}" = install ]; then
    echo "error: export compliance: REINS_EXPORT_COMPLIANCE_CODE is not set. An archive for App Store Connect needs the code of the app's approved encryption declaration (docs/ios-app-store-submission.md, Export compliance). Set it in Config/Local.xcconfig or with -xcconfig; REINS_EXPORT_COMPLIANCE_REQUIRED = NO archives without it (such a build cannot be distributed until its compliance is answered in App Store Connect)."
    exit 1
else
    echo "export compliance: no REINS_EXPORT_COMPLIANCE_CODE, ITSEncryptionExportComplianceCode left out (fine for development builds)"
fi

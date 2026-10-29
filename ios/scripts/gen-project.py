#!/usr/bin/env python3
"""Writes ios/Reins.xcodeproj from the description below. Folders are Xcode "synchronized" groups, so new source and
resource files are picked up without touching the project; only targets, folders per target, build settings,
packages and privacy manifests live here. Each target's PrivacyInfo.xcprivacy is an explicit resource so it lands at
that bundle's root (a shared file in a synchronized folder would be copied into every target that compiles the
folder). ONNX Runtime's embedded framework gets its manifest from scripts/fix-embedded-frameworks.sh. Re-run after
changing this file:

    ios/scripts/gen-project.py
"""

import hashlib
import os
import plistlib
import sys
from pathlib import Path

IOS = Path(__file__).resolve().parent.parent
PROJECT = IOS / "Reins.xcodeproj"
DEPLOYMENT = "26.0"
BUNDLE = "com.reins2fa.app"
APP_GROUP = "group.com.reins2fa.app"

CORE_LINK = {
    "LIBRARY_SEARCH_PATHS[sdk=iphoneos*]": "$(SRCROOT)/../target/ios-core/iphoneos",
    "LIBRARY_SEARCH_PATHS[sdk=iphonesimulator*]": "$(SRCROOT)/../target/ios-core/iphonesimulator",
    "SWIFT_INCLUDE_PATHS[sdk=iphoneos*]": "$(SRCROOT)/../target/ios-core/iphoneos/include",
    "SWIFT_INCLUDE_PATHS[sdk=iphonesimulator*]": "$(SRCROOT)/../target/ios-core/iphonesimulator/include",
    "OTHER_LDFLAGS": ["-lreins_core", "-lc++", "-framework", "Security", "-framework", "SystemConfiguration"],
}

# Swift packages: name -> (url, requirement dict, [products]).
PACKAGES = {
    "onnxruntime-swift-package-manager": (
        "https://github.com/microsoft/onnxruntime-swift-package-manager",
        {"kind": "upToNextMajorVersion", "minimumVersion": "1.20.0"},
        ["onnxruntime"],
    ),
}

# ONNX Runtime's C API as a Clang module (the package's Objective-C wrapper has no boolean tensors).
ORT_C = "$(SRCROOT)/Vendor/OnnxRuntimeC"

TARGETS = [
    {
        "name": "Reins",
        "type": "com.apple.product-type.application",
        "product": "Reins.app",
        "folders": ["Reins", "Shared", "Core"],
        "plist_exceptions": {"Reins": ["Info.plist"]},
        "core": True,
        "embed": ["ReinsWidgets", "ReinsNotifications"],
        "packages": ["onnxruntime"],
        "fix_frameworks": True,
        "swift_includes": [ORT_C],
        "settings": {
            "INFOPLIST_FILE": "Reins/Info.plist",
            "CODE_SIGN_ENTITLEMENTS": "Reins/Reins.entitlements",
            "PRODUCT_BUNDLE_IDENTIFIER": BUNDLE,
            "ASSETCATALOG_COMPILER_APPICON_NAME": "AppIcon",
            "ASSETCATALOG_COMPILER_GLOBAL_ACCENT_COLOR_NAME": "AccentColor",
            "TARGETED_DEVICE_FAMILY": "1,2",
            "SWIFT_ACTIVE_COMPILATION_CONDITIONS": "REINS_APP $(inherited)",
            "LD_RUNPATH_SEARCH_PATHS": ["$(inherited)", "@executable_path/Frameworks"],
        },
    },
    {
        "name": "ReinsWidgets",
        "type": "com.apple.product-type.app-extension",
        "product": "ReinsWidgets.appex",
        "folders": ["ReinsWidgets", "Shared"],
        "plist_exceptions": {"ReinsWidgets": ["Info.plist"]},
        "settings": {
            "INFOPLIST_FILE": "ReinsWidgets/Info.plist",
            "CODE_SIGN_ENTITLEMENTS": "ReinsWidgets/ReinsWidgets.entitlements",
            "PRODUCT_BUNDLE_IDENTIFIER": BUNDLE + ".widgets",
            "TARGETED_DEVICE_FAMILY": "1,2",
            "SKIP_INSTALL": "YES",
            "SWIFT_ACTIVE_COMPILATION_CONDITIONS": "REINS_WIDGETS $(inherited)",
            "LD_RUNPATH_SEARCH_PATHS": ["$(inherited)", "@executable_path/Frameworks", "@executable_path/../../Frameworks"],
        },
    },
    {
        "name": "ReinsNotifications",
        "type": "com.apple.product-type.app-extension",
        "product": "ReinsNotifications.appex",
        "folders": ["ReinsNotifications", "Shared", "Core"],
        "plist_exceptions": {"ReinsNotifications": ["Info.plist"]},
        "core": True,
        "settings": {
            "INFOPLIST_FILE": "ReinsNotifications/Info.plist",
            "CODE_SIGN_ENTITLEMENTS": "ReinsNotifications/ReinsNotifications.entitlements",
            "PRODUCT_BUNDLE_IDENTIFIER": BUNDLE + ".notifications",
            "TARGETED_DEVICE_FAMILY": "1,2",
            "SKIP_INSTALL": "YES",
            "SWIFT_ACTIVE_COMPILATION_CONDITIONS": "REINS_EXTENSION $(inherited)",
            "LD_RUNPATH_SEARCH_PATHS": ["$(inherited)", "@executable_path/Frameworks", "@executable_path/../../Frameworks"],
        },
    },
    {
        "name": "ReinsTests",
        "type": "com.apple.product-type.bundle.unit-test",
        "product": "ReinsTests.xctest",
        "folders": ["ReinsTests"],
        "host": "Reins",
        "swift_includes": [ORT_C],
        "settings": {
            "PRODUCT_BUNDLE_IDENTIFIER": BUNDLE + ".tests",
            "GENERATE_INFOPLIST_FILE": "YES",
            "TEST_HOST": "$(BUILT_PRODUCTS_DIR)/Reins.app/$(BUNDLE_EXECUTABLE_FOLDER_PATH)/Reins",
            "BUNDLE_LOADER": "$(TEST_HOST)",
            "TARGETED_DEVICE_FAMILY": "1,2",
            "SWIFT_INCLUDE_PATHS[sdk=iphoneos*]": "$(SRCROOT)/../target/ios-core/iphoneos/include",
            "SWIFT_INCLUDE_PATHS[sdk=iphonesimulator*]": "$(SRCROOT)/../target/ios-core/iphonesimulator/include",
        },
    },
    {
        "name": "ReinsUITests",
        "type": "com.apple.product-type.bundle.ui-testing",
        "product": "ReinsUITests.xctest",
        "folders": ["ReinsUITests"],
        "host": "Reins",
        "ui_tests": True,
        "settings": {
            "PRODUCT_BUNDLE_IDENTIFIER": BUNDLE + ".uitests",
            "GENERATE_INFOPLIST_FILE": "YES",
            "TEST_TARGET_NAME": "Reins",
            "TARGETED_DEVICE_FAMILY": "1,2",
        },
    },
]

COMMON = {
    "ALWAYS_SEARCH_USER_PATHS": "NO",
    "CLANG_ENABLE_MODULES": "YES",
    "CLANG_ENABLE_OBJC_ARC": "YES",
    "ENABLE_STRICT_OBJC_MSGSEND": "YES",
    "ENABLE_USER_SCRIPT_SANDBOXING": "NO",
    "GCC_C_LANGUAGE_STANDARD": "gnu17",
    "IPHONEOS_DEPLOYMENT_TARGET": DEPLOYMENT,
    "SDKROOT": "iphoneos",
    "SUPPORTED_PLATFORMS": "iphoneos iphonesimulator",
    "SUPPORTS_MACCATALYST": "NO",
    "SWIFT_VERSION": "5.0",
    "SWIFT_EMIT_LOC_STRINGS": "YES",
    "CODE_SIGN_STYLE": "Automatic",
    "DEVELOPMENT_TEAM": "$(REINS_TEAM)",
    "MARKETING_VERSION": "$(REINS_VERSION)",
    "CURRENT_PROJECT_VERSION": "$(REINS_BUILD_NUMBER)",
    "ENABLE_PREVIEWS": "YES",
    # The Rust core is built for Apple silicon only (aarch64-apple-ios, aarch64-apple-ios-sim).
    "EXCLUDED_ARCHS[sdk=iphonesimulator*]": "x86_64",
}
DEBUG = {
    "DEBUG_INFORMATION_FORMAT": "dwarf",
    "ENABLE_TESTABILITY": "YES",
    "GCC_OPTIMIZATION_LEVEL": "0",
    "GCC_PREPROCESSOR_DEFINITIONS": ["DEBUG=1", "$(inherited)"],
    "ONLY_ACTIVE_ARCH": "YES",
    "SWIFT_ACTIVE_COMPILATION_CONDITIONS": "DEBUG $(inherited)",
    "SWIFT_OPTIMIZATION_LEVEL": "-Onone",
}
RELEASE = {
    "DEBUG_INFORMATION_FORMAT": "dwarf-with-dsym",
    "ENABLE_NS_ASSERTIONS": "NO",
    "SWIFT_COMPILATION_MODE": "wholemodule",
    "SWIFT_OPTIMIZATION_LEVEL": "-O",
    "VALIDATE_PRODUCT": "YES",
}

# One manifest per bundle, at ios/Privacy/<name>/PrivacyInfo.xcprivacy. The declarations live in those plists.
# OnnxRuntime is not a target: fix-embedded-frameworks.sh copies it into the embedded onnxruntime.framework, which
# references stat/fstat and ships no manifest of its own.
PRIVACY_TARGETS = [t["name"] for t in TARGETS]
ONNX_PRIVACY = "OnnxRuntime"

# Apple's required-reason categories and the reason codes that are valid for each. Kept here so a typo fails
# generation instead of App Store validation.
PRIVACY_API_REASONS = {
    "NSPrivacyAccessedAPICategoryFileTimestamp": {"DDA9.1", "C617.1", "3B52.1", "0A2A.1"},
    "NSPrivacyAccessedAPICategorySystemBootTime": {"35F9.1", "8FFB.1", "3D61.1"},
    "NSPrivacyAccessedAPICategoryDiskSpace": {"85F4.1", "E174.1", "7D9E.1", "B728.1"},
    "NSPrivacyAccessedAPICategoryActiveKeyboards": {"3EC4.1", "54BD.1"},
    "NSPrivacyAccessedAPICategoryUserDefaults": {"CA92.1", "1C8F.1", "C56D.1", "AC6B.1"},
}
PRIVACY_DATA_TYPES = {
    "NSPrivacyCollectedDataTypeName",
    "NSPrivacyCollectedDataTypeEmailAddress",
    "NSPrivacyCollectedDataTypePhoneNumber",
    "NSPrivacyCollectedDataTypePhysicalAddress",
    "NSPrivacyCollectedDataTypeOtherUserContactInfo",
    "NSPrivacyCollectedDataTypeHealth",
    "NSPrivacyCollectedDataTypeFitness",
    "NSPrivacyCollectedDataTypePaymentInfo",
    "NSPrivacyCollectedDataTypeCreditInfo",
    "NSPrivacyCollectedDataTypeOtherFinancialInfo",
    "NSPrivacyCollectedDataTypePreciseLocation",
    "NSPrivacyCollectedDataTypeCoarseLocation",
    "NSPrivacyCollectedDataTypeSensitiveInfo",
    "NSPrivacyCollectedDataTypeContacts",
    "NSPrivacyCollectedDataTypeEmailsOrTextMessages",
    "NSPrivacyCollectedDataTypePhotosorVideos",
    "NSPrivacyCollectedDataTypeAudioData",
    "NSPrivacyCollectedDataTypeGameplayContent",
    "NSPrivacyCollectedDataTypeCustomerSupport",
    "NSPrivacyCollectedDataTypeOtherUserContent",
    "NSPrivacyCollectedDataTypeBrowsingHistory",
    "NSPrivacyCollectedDataTypeSearchHistory",
    "NSPrivacyCollectedDataTypeUserID",
    "NSPrivacyCollectedDataTypeDeviceID",
    "NSPrivacyCollectedDataTypePurchaseHistory",
    "NSPrivacyCollectedDataTypeProductInteraction",
    "NSPrivacyCollectedDataTypeAdvertisingData",
    "NSPrivacyCollectedDataTypeOtherUsageData",
    "NSPrivacyCollectedDataTypeCrashData",
    "NSPrivacyCollectedDataTypePerformanceData",
    "NSPrivacyCollectedDataTypeOtherDiagnosticData",
    "NSPrivacyCollectedDataTypeOtherDataTypes",
}
PRIVACY_PURPOSES = {
    "NSPrivacyCollectedDataTypePurposeThirdPartyAdvertising",
    "NSPrivacyCollectedDataTypePurposeDeveloperAdvertising",
    "NSPrivacyCollectedDataTypePurposeAnalytics",
    "NSPrivacyCollectedDataTypePurposeProductPersonalization",
    "NSPrivacyCollectedDataTypePurposeAppFunctionality",
    "NSPrivacyCollectedDataTypePurposeOther",
}


def privacy_manifest(name: str) -> Path:
    return IOS / "Privacy" / name / "PrivacyInfo.xcprivacy"


def check_privacy_manifests() -> None:
    """Reject a manifest App Store Connect would reject, before writing the project."""
    for name in [*PRIVACY_TARGETS, ONNX_PRIVACY]:
        path = privacy_manifest(name)
        if not path.is_file():
            sys.exit(f"error: missing privacy manifest {path.relative_to(IOS)}")
        try:
            data = plistlib.loads(path.read_bytes())
        except (plistlib.InvalidFileException, ValueError) as e:
            sys.exit(f"error: {path.relative_to(IOS)} is not a plist ({e})")
        if not isinstance(data, dict):
            sys.exit(f"error: {path.relative_to(IOS)} is not a dictionary")
        tracking = data.get("NSPrivacyTracking")
        if not isinstance(tracking, bool):
            sys.exit(f"error: {path.relative_to(IOS)} NSPrivacyTracking must be a boolean")
        domains = data.get("NSPrivacyTrackingDomains")
        if not isinstance(domains, list) or (tracking is False and domains):
            sys.exit(f"error: {path.relative_to(IOS)} tracking domains must be a list, empty when not tracking")
        apis = data.get("NSPrivacyAccessedAPITypes")
        collected = data.get("NSPrivacyCollectedDataTypes")
        if not isinstance(apis, list) or not isinstance(collected, list):
            sys.exit(f"error: {path.relative_to(IOS)} needs NSPrivacyAccessedAPITypes and NSPrivacyCollectedDataTypes")
        seen = set()
        for entry in apis:
            if not isinstance(entry, dict):
                sys.exit(f"error: {path.relative_to(IOS)} has a non-dictionary accessed-API entry")
            category = entry.get("NSPrivacyAccessedAPIType")
            reasons = entry.get("NSPrivacyAccessedAPITypeReasons")
            if category not in PRIVACY_API_REASONS or not isinstance(reasons, list) or not reasons:
                sys.exit(f"error: {path.relative_to(IOS)} has an unknown API category {category!r}")
            if category in seen:
                sys.exit(f"error: {path.relative_to(IOS)} repeats {category}")
            seen.add(category)
            unknown = [r for r in reasons if r not in PRIVACY_API_REASONS[category]]
            if unknown:
                sys.exit(f"error: {path.relative_to(IOS)} {category} has unknown reason codes {unknown}")
        for entry in collected:
            if not isinstance(entry, dict):
                sys.exit(f"error: {path.relative_to(IOS)} has a non-dictionary collected-data entry")
            kind = entry.get("NSPrivacyCollectedDataType")
            if kind not in PRIVACY_DATA_TYPES:
                sys.exit(f"error: {path.relative_to(IOS)} has an unknown data type {kind!r}")
            if not isinstance(entry.get("NSPrivacyCollectedDataTypeLinked"), bool):
                sys.exit(f"error: {path.relative_to(IOS)} {kind} needs a boolean NSPrivacyCollectedDataTypeLinked")
            if not isinstance(entry.get("NSPrivacyCollectedDataTypeTracking"), bool):
                sys.exit(f"error: {path.relative_to(IOS)} {kind} needs a boolean NSPrivacyCollectedDataTypeTracking")
            purposes = entry.get("NSPrivacyCollectedDataTypePurposes")
            if not isinstance(purposes, list) or not purposes or any(p not in PRIVACY_PURPOSES for p in purposes):
                sys.exit(f"error: {path.relative_to(IOS)} {kind} has invalid purposes {purposes!r}")


def oid(*parts: str) -> str:
    return hashlib.sha1("/".join(parts).encode()).hexdigest()[:24].upper()


def q(value: str) -> str:
    if value and all(c.isalnum() or c in "._/" for c in value) and not value[0].isdigit():
        return value
    return '"' + value.replace("\\", "\\\\").replace('"', '\\"').replace("\n", "\\n") + '"'


def fmt(value, indent: int) -> str:
    pad = "\t" * indent
    if isinstance(value, dict):
        lines = ["{"]
        for k, v in value.items():
            lines.append(f"{pad}\t{q(k)} = {fmt(v, indent + 1)};")
        lines.append(pad + "}")
        return "\n".join(lines)
    if isinstance(value, list):
        lines = ["("]
        for v in value:
            lines.append(f"{pad}\t{fmt(v, indent + 1)},")
        lines.append(pad + ")")
        return "\n".join(lines)
    return q(str(value))


def main() -> None:
    check_privacy_manifests()
    objects: dict[str, dict] = {}
    project_id = oid("project")
    main_group = oid("group", "main")
    products_group = oid("group", "products")
    folder_ids: dict[str, str] = {}

    all_folders = []
    for t in TARGETS:
        for f in t["folders"]:
            if f not in all_folders:
                all_folders.append(f)

    for f in all_folders:
        exceptions = []
        for t in TARGETS:
            excluded = t.get("plist_exceptions", {}).get(f)
            if excluded:
                eid = oid("exceptions", f, t["name"])
                objects[eid] = {
                    "isa": "PBXFileSystemSynchronizedBuildFileExceptionSet",
                    "membershipExceptions": excluded,
                    "target": oid("target", t["name"]),
                }
                exceptions.append(eid)
        gid = oid("folder", f)
        folder_ids[f] = gid
        entry = {"isa": "PBXFileSystemSynchronizedRootGroup"}
        if exceptions:
            entry["exceptions"] = exceptions
        entry.update({"path": f, "sourceTree": "<group>"})
        objects[gid] = entry

    # Privacy manifests sit outside the synchronized folders. The file reference's path is the filename, and its
    # group carries the directory, so Copy Bundle Resources places PrivacyInfo.xcprivacy at the bundle root.
    privacy_children = []
    for name in [*PRIVACY_TARGETS, ONNX_PRIVACY]:
        fref = oid("file", "privacy", name)
        objects[fref] = {
            "isa": "PBXFileReference",
            "lastKnownFileType": "text.plist.xml",
            "path": "PrivacyInfo.xcprivacy",
            "sourceTree": "<group>",
        }
        sub = oid("group", "privacy", name)
        objects[sub] = {
            "isa": "PBXGroup",
            "children": [fref],
            "path": name,
            "sourceTree": "<group>",
        }
        privacy_children.append(sub)
    privacy_group = oid("group", "privacy")
    objects[privacy_group] = {
        "isa": "PBXGroup",
        "children": privacy_children,
        "path": "Privacy",
        "sourceTree": "<group>",
    }

    package_refs = {}
    for name, (url, requirement, _products) in PACKAGES.items():
        pid = oid("package", name)
        package_refs[name] = pid
        objects[pid] = {"isa": "XCRemoteSwiftPackageReference", "repositoryURL": url, "requirement": requirement}
    product_package = {p: name for name, (_u, _r, prods) in PACKAGES.items() for p in prods}

    target_ids = []
    for t in TARGETS:
        name = t["name"]
        tid = oid("target", name)
        target_ids.append(tid)
        product_ref = oid("product", name)
        ext = t["product"].rsplit(".", 1)[1]
        file_type = {"app": "wrapper.application", "appex": "wrapper.app-extension", "xctest": "wrapper.cfbundle"}[ext]
        objects[product_ref] = {
            "isa": "PBXFileReference",
            "explicitFileType": file_type,
            "includeInIndex": "0",
            "path": t["product"],
            "sourceTree": "BUILT_PRODUCTS_DIR",
        }
        phases = []
        if t.get("core"):
            sh = oid("phase", name, "core")
            objects[sh] = {
                "isa": "PBXShellScriptBuildPhase",
                "alwaysOutOfDate": "1",
                "buildActionMask": "2147483647",
                "files": [],
                "inputPaths": [],
                "name": "Rust core",
                "outputPaths": [],
                "runOnlyForDeploymentPostprocessing": "0",
                "shellPath": "/bin/sh",
                "shellScript": 'exec "$SRCROOT/scripts/build-core.sh"\n',
                "showEnvVarsInLog": "0",
            }
            phases.append(sh)
        for kind in ("Sources", "Frameworks", "Resources"):
            pid = oid("phase", name, kind)
            files = []
            if kind == "Frameworks":
                for product in t.get("packages", []):
                    dep = oid("pkgdep", name, product)
                    objects[dep] = {
                        "isa": "XCSwiftPackageProductDependency",
                        "package": package_refs[product_package[product]],
                        "productName": product,
                    }
                    bf = oid("buildfile", name, product)
                    objects[bf] = {"isa": "PBXBuildFile", "productRef": dep}
                    files.append(bf)
            if kind == "Resources":
                # The ONNX manifest is not a resource of any target; the embed script copies it into the framework.
                bf = oid("buildfile", name, "privacy")
                objects[bf] = {"isa": "PBXBuildFile", "fileRef": oid("file", "privacy", name)}
                files.append(bf)
            objects[pid] = {
                "isa": f"PBX{kind}BuildPhase",
                "buildActionMask": "2147483647",
                "files": files,
                "runOnlyForDeploymentPostprocessing": "0",
            }
            phases.append(pid)
        deps = []
        if t.get("embed"):
            files = []
            for ext_name in t["embed"]:
                bf = oid("embed", name, ext_name)
                objects[bf] = {
                    "isa": "PBXBuildFile",
                    "fileRef": oid("product", ext_name),
                    "settings": {"ATTRIBUTES": ["RemoveHeadersOnCopy"]},
                }
                files.append(bf)
                deps.append(oid("dep", name, ext_name))
            cp = oid("phase", name, "embed")
            objects[cp] = {
                "isa": "PBXCopyFilesBuildPhase",
                "buildActionMask": "2147483647",
                "dstPath": "",
                "dstSubfolderSpec": "13",
                "files": files,
                "name": "Embed Foundation Extensions",
                "runOnlyForDeploymentPostprocessing": "0",
            }
            phases.append(cp)
        if t.get("fix_frameworks"):
            # Last, once the package frameworks are embedded: scripts/fix-embedded-frameworks.sh says why.
            sh = oid("phase", name, "fix-frameworks")
            objects[sh] = {
                "isa": "PBXShellScriptBuildPhase",
                "alwaysOutOfDate": "1",
                "buildActionMask": "2147483647",
                "files": [],
                "inputPaths": [],
                "name": "Embedded frameworks' Info.plist",
                "outputPaths": [],
                "runOnlyForDeploymentPostprocessing": "0",
                "shellPath": "/bin/sh",
                "shellScript": 'exec "$SRCROOT/scripts/fix-embedded-frameworks.sh"\n',
                "showEnvVarsInLog": "0",
            }
            phases.append(sh)
        if t.get("host"):
            deps.append(oid("dep", name, t["host"]))
        for dep_target in [*(t.get("embed") or []), *([t["host"]] if t.get("host") else [])]:
            dep = oid("dep", name, dep_target)
            proxy = oid("proxy", name, dep_target)
            objects[proxy] = {
                "isa": "PBXContainerItemProxy",
                "containerPortal": project_id,
                "proxyType": "1",
                "remoteGlobalIDString": oid("target", dep_target),
                "remoteInfo": dep_target,
            }
            objects[dep] = {"isa": "PBXTargetDependency", "target": oid("target", dep_target), "targetProxy": proxy}

        settings = dict(t["settings"])
        if t.get("core"):
            settings.update(CORE_LINK)
        # Extra Clang modules for Swift (ONNX Runtime's C API), next to the core's.
        for key in [k for k in settings if k.startswith("SWIFT_INCLUDE_PATHS")]:
            settings[key] = " ".join([settings[key], *t.get("swift_includes", [])])
        configs = []
        for cfg, extra in (("Debug", {}), ("Release", {})):
            cid = oid("config", name, cfg)
            merged = dict(settings)
            merged.update(extra)
            if "SWIFT_ACTIVE_COMPILATION_CONDITIONS" in merged and cfg == "Debug":
                merged["SWIFT_ACTIVE_COMPILATION_CONDITIONS"] = merged["SWIFT_ACTIVE_COMPILATION_CONDITIONS"].replace(
                    "$(inherited)", "DEBUG $(inherited)"
                )
            merged.setdefault("PRODUCT_NAME", "$(TARGET_NAME)")
            objects[cid] = {
                "isa": "XCBuildConfiguration",
                "baseConfigurationReference": oid("xcconfig"),
                "buildSettings": dict(sorted(merged.items())),
                "name": cfg,
            }
            configs.append(cid)
        clist = oid("configlist", name)
        objects[clist] = {
            "isa": "XCConfigurationList",
            "buildConfigurations": configs,
            "defaultConfigurationIsVisible": "0",
            "defaultConfigurationName": "Release",
        }
        target = {
            "isa": "PBXNativeTarget",
            "buildConfigurationList": clist,
            "buildPhases": phases,
            "buildRules": [],
            "dependencies": deps,
            "fileSystemSynchronizedGroups": [folder_ids[f] for f in t["folders"]],
            "name": name,
            "packageProductDependencies": [oid("pkgdep", name, p) for p in t.get("packages", [])],
            "productName": name,
            "productReference": product_ref,
            "productType": t["type"],
        }
        objects[tid] = target

    xcconfig = oid("xcconfig")
    objects[xcconfig] = {
        "isa": "PBXFileReference",
        "lastKnownFileType": "text.xcconfig",
        "path": "Config/Reins.xcconfig",
        "sourceTree": "<group>",
    }
    objects[main_group] = {
        "isa": "PBXGroup",
        "children": [xcconfig, privacy_group, *[folder_ids[f] for f in all_folders], products_group],
        "sourceTree": "<group>",
    }
    objects[products_group] = {
        "isa": "PBXGroup",
        "children": [oid("product", t["name"]) for t in TARGETS],
        "name": "Products",
        "sourceTree": "<group>",
    }
    proj_configs = []
    for cfg, extra in (("Debug", DEBUG), ("Release", RELEASE)):
        cid = oid("config", "project", cfg)
        merged = dict(COMMON)
        merged.update(extra)
        objects[cid] = {
            "isa": "XCBuildConfiguration",
            "baseConfigurationReference": xcconfig,
            "buildSettings": dict(sorted(merged.items())),
            "name": cfg,
        }
        proj_configs.append(cid)
    proj_list = oid("configlist", "project")
    objects[proj_list] = {
        "isa": "XCConfigurationList",
        "buildConfigurations": proj_configs,
        "defaultConfigurationIsVisible": "0",
        "defaultConfigurationName": "Release",
    }
    objects[project_id] = {
        "isa": "PBXProject",
        "attributes": {
            "BuildIndependentTargetsInParallel": "1",
            "LastSwiftUpdateCheck": "2710",
            "LastUpgradeCheck": "2710",
        },
        "buildConfigurationList": proj_list,
        "developmentRegion": "en",
        "hasScannedForEncodings": "0",
        "knownRegions": ["en", "Base"],
        "mainGroup": main_group,
        "minimizedProjectReferenceProxies": "1",
        "packageReferences": list(package_refs.values()),
        "preferredProjectObjectVersion": "77",
        "productRefGroup": products_group,
        "projectDirPath": "",
        "projectRoot": "",
        "targets": target_ids,
    }

    by_isa: dict[str, list[tuple[str, dict]]] = {}
    for k, v in objects.items():
        by_isa.setdefault(v["isa"], []).append((k, v))
    out = ["// !$*UTF8*$!", "{", "\tarchiveVersion = 1;", "\tclasses = {", "\t};", "\tobjectVersion = 77;", "\tobjects = {"]
    for isa in sorted(by_isa):
        out.append(f"\n/* Begin {isa} section */")
        for k, v in sorted(by_isa[isa]):
            body = {"isa": v["isa"], **{kk: vv for kk, vv in v.items() if kk != "isa"}}
            out.append(f"\t\t{k} = {fmt(body, 2)};")
        out.append(f"/* End {isa} section */")
    out += ["\t};", f"\trootObject = {project_id};", "}", ""]
    PROJECT.mkdir(exist_ok=True)
    (PROJECT / "project.pbxproj").write_text("\n".join(out))
    write_schemes(project_id)


def write_schemes(project_id: str) -> None:
    schemes = PROJECT / "xcshareddata" / "xcschemes"
    schemes.mkdir(parents=True, exist_ok=True)

    def ref(name: str, product: str) -> str:
        return (
            f'<BuildableReference BuildableIdentifier = "primary" BlueprintIdentifier = "{oid("target", name)}" '
            f'BuildableName = "{product}" BlueprintName = "{name}" ReferencedContainer = "container:Reins.xcodeproj">'
            "</BuildableReference>"
        )

    app = ref("Reins", "Reins.app")
    tests = "".join(
        f'<TestableReference skipped = "NO" parallelizable = "NO">{ref(n, n + ".xctest")}</TestableReference>'
        for n in ("ReinsTests", "ReinsUITests")
    )
    scheme = f"""<?xml version="1.0" encoding="UTF-8"?>
<Scheme LastUpgradeVersion = "2710" version = "1.7">
   <BuildAction parallelizeBuildables = "YES" buildImplicitDependencies = "YES">
      <BuildActionEntries>
         <BuildActionEntry buildForTesting = "YES" buildForRunning = "YES" buildForProfiling = "YES" buildForArchiving = "YES" buildForAnalyzing = "YES">
            {app}
         </BuildActionEntry>
      </BuildActionEntries>
   </BuildAction>
   <TestAction buildConfiguration = "Debug" selectedDebuggerIdentifier = "Xcode.DebuggerFoundation.Debugger.LLDB" selectedLauncherIdentifier = "Xcode.DebuggerFoundation.Launcher.LLDB" shouldUseLaunchSchemeArgsEnv = "YES">
      <Testables>{tests}</Testables>
   </TestAction>
   <LaunchAction buildConfiguration = "Debug" selectedDebuggerIdentifier = "Xcode.DebuggerFoundation.Debugger.LLDB" selectedLauncherIdentifier = "Xcode.DebuggerFoundation.Launcher.LLDB" launchStyle = "0" useCustomWorkingDirectory = "NO" ignoresPersistentStateOnLaunch = "NO" debugDocumentVersioning = "YES" debugServiceExtension = "internal" allowLocationSimulation = "YES">
      <BuildableProductRunnable runnableDebuggingMode = "0">
         {app}
      </BuildableProductRunnable>
   </LaunchAction>
   <ProfileAction buildConfiguration = "Release" shouldUseLaunchSchemeArgsEnv = "YES" savedToolIdentifier = "" useCustomWorkingDirectory = "NO" debugDocumentVersioning = "YES">
      <BuildableProductRunnable runnableDebuggingMode = "0">
         {app}
      </BuildableProductRunnable>
   </ProfileAction>
   <AnalyzeAction buildConfiguration = "Debug"></AnalyzeAction>
   <ArchiveAction buildConfiguration = "Release" revealArchiveInOrganizer = "YES"></ArchiveAction>
</Scheme>
"""
    (schemes / "Reins.xcscheme").write_text(scheme)


if __name__ == "__main__":
    os.chdir(IOS)
    main()

# Aegis

Aegis is a native browser runtime and automation stack for deterministic, host-backed web execution.

## What Is Here

- Rust application and runtime code in `src/`
- Native macOS browser host and standalone app in `native/`
- Tests in `tests/`
- Bundled assets in `assets/`
- Vendored browser dependencies in `third_party/`

## Current State

- Standalone macOS browser app launches headfully
- Native Cocoa browser shell is wired to the embedded CEF browser
- Deterministic runtime and trace/test flows are included in this repo

## Development

Native browser dependency:

- The macOS native browser build expects the CEF SDK at `third_party/cef/cef_binary_146.0.6+g68649e2+chromium-146.0.7680.154_macosarm64`.
- That SDK is intentionally not tracked in git because the binary payload exceeds GitHub limits.
- Provision it locally before running native configure/build steps.

Rust tests:

```bash
cargo test
```

Native macOS app build:

```bash
xcodebuild -project native/build-xcode/aegis_native.xcodeproj \
  -scheme aegis_native \
  -configuration Debug \
  -arch arm64 \
  CODE_SIGNING_ALLOWED=NO \
  CODE_SIGNING_REQUIRED=NO \
  CODE_SIGN_IDENTITY= \
  build
```

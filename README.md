# Eclipse UU runtime plugin

This repository builds and distributes the optional UU native library. The main Eclipse application has no dependency on this repository and ships without this plugin.

Download a plugin ZIP matching your application's SDK, extract it at the application directory, rescan in Settings → Plugins, and explicitly enable it. New and upgraded installations default to disabled. The archive creates `plugins/uu/plugin.json` and the plugin DLL. It contains no account state and no application executables.

The plugin owns QR login, account devices, favorites, assistance requests, rendezvous, the UU controller protocol and the optional official-client host adapter. Settings and accounts live under the application's data directory in `plugins/uu/`. Legacy account migration occurs only after explicit enablement, preserving originals. Official relay defaults off; the supported optional relay transport is TURN/UDP. TURN/TCP and TURN/TLS are not implemented. Official mobile desktop interoperability still requires real-device acceptance; generated media tests do not establish it.

## Modules

After enabling the plugin, select its three modules independently in Settings → Plugins. The compatibility client and account connection service default on; the experimental compatibility server defaults off. Explicit module choices persist across restarts and upgrades. The client adds the “UU compatibility client” toolbar entry, the account service supplies native Eclipse rendezvous and account settings, and the experimental server exposes its permissions directly in Service management. Their shared transport does not publish a disabled native entrance. Disabling a module preserves existing streams; restart hosting to apply changed entrance settings and permissions.

The compatibility server module controls official UU connection admission. The legacy enable flag in `host.json` does not enable that module; control, audio and clipboard permissions remain in that file. Remote assistance contains only the official device ID and verification-code flow. Native pairing stays in Eclipse's own device and service pages.

## Build

Read `engine-sdk.json`, check out its exact engine revision, and place this repository at `<engine>/plugins`. It is a separate Cargo workspace with its own lock. Use the pinned Rust toolchain and matching target/features; this native SDK does not promise a stable Rust ABI.

```sh
cd plugins
cargo test --release --locked -p uu-compat --tests
python3 tools/package.py --target x86_64-pc-windows-gnu
```

The main application provides the shared engine SDK and compiler runtime. Do not replace them with a plugin build's copies. Loaded libraries stay mapped until application exit; disablement prevents new requests and does not unload live session code.

Plugin releases are published only from this repository. The main application's Release contains only its complete main package. License notices cover the plugin and its engine dependencies; splitting repositories does not remove third-party obligations.

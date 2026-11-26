# ciadpi / ByeDPI on macOS

Sidestep embeds a precompiled Universal 2 Mach-O `ciadpi-darwin` binary (`arm64` + `x86_64`) so both Apple Silicon and Intel Macs get a zero-dependency install. This binary is automatically extracted to `~/Library/Application Support/Sidestep/ciadpi` (or `Sidestep-data/ciadpi` when `sidestep.portable` is present) on first run.

## Why you might replace the embedded binary

| Situation | Effect |
|---|---|
| Homebrew / source-only packaging | Policy forbids vendored precompiled binaries — build without `vendor/ciadpi-darwin` |
| You prefer to audit what runs | Build `ciadpi` from source and place it on your `PATH` |

## Using a system-installed ciadpi

If `ciadpi` is already installed on your system (`/opt/homebrew/bin/ciadpi`, `/usr/local/bin/ciadpi`, `/usr/bin/ciadpi`, or on `PATH`), Sidestep finds and uses it automatically — the embedded binary is never extracted. No configuration needed.

```sh
git clone --depth 1 https://github.com/hufrea/byedpi
cd byedpi && make
sudo install -m755 ciadpi /usr/local/bin/ciadpi
```

## For packagers (Homebrew / source-only builds)

Do **not** include `vendor/ciadpi-darwin` in your package. Instead:

1. Declare `byedpi` (or build `ciadpi` from source) as a runtime dependency.
2. Build Sidestep without the vendored binary by replacing `src/setup/macos.rs`'s `PAYLOAD` constant with an empty slice:
   ```rust
   pub const PAYLOAD: [(&str, &[u8]); 0] = [];
   ```
   The runtime path (`find_system_ciadpi()`) will then locate the system binary.
3. The build will still compile cleanly — `include_bytes!` on the `ciadpi-darwin` path only runs if `PAYLOAD` references it.

## Building Universal 2 `ciadpi-darwin` from source

```sh
git clone --depth 1 https://github.com/hufrea/byedpi
cd byedpi
make CC="clang -arch arm64 -mmacosx-version-min=11.0" clean all && mv ciadpi ciadpi-arm64
make CC="clang -arch x86_64 -mmacosx-version-min=11.0" clean all && mv ciadpi ciadpi-x86_64
lipo -create ciadpi-arm64 ciadpi-x86_64 -output /path/to/Sidestep/vendor/ciadpi-darwin
chmod +x /path/to/Sidestep/vendor/ciadpi-darwin
```

After rebuilding Sidestep (`cargo build --release`), `sidestep` embeds the updated Universal 2 binary automatically.

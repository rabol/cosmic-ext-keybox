# Name of the application's binary.
name := 'cosmic-ext-keybox'
# The unique ID of the application.
appid := 'dev.rabol.Keybox'

# Path to root file system, which defaults to `/`.
rootdir := ''
# The prefix for the `/usr` directory.
prefix := '/usr'
# The location of the cargo target directory.
cargo-target-dir := env('CARGO_TARGET_DIR', 'target')

# Application's appstream metadata
appdata := appid + '.metainfo.xml'
# Application's desktop entry
desktop := appid + '.desktop'
# Application's icon.
icon-svg := appid + '.svg'

# Install destinations
base-dir := absolute_path(clean(rootdir / prefix))
appdata-dst := base-dir / 'share' / 'metainfo' / appdata
bin-dst := base-dir / 'bin' / name
desktop-dst := base-dir / 'share' / 'applications' / desktop
icons-dst := base-dir / 'share' / 'icons' / 'hicolor'
icon-svg-dst := icons-dst / 'scalable' / 'apps' / icon-svg

# Default recipe which runs `just build-release`
default: build-release

# Runs `cargo clean`
clean:
    cargo clean

# Compiles with debug profile
build-debug *args:
    cargo build --locked {{args}}

# Compiles with release profile
build-release *args: (build-debug '--release' args)

# Runs clippy and the test suite
check *args:
    cargo fmt --check
    cargo clippy --all-targets --locked {{args}} -- -W clippy::pedantic
    cargo test --locked

# Run the application against the real session keyring
run *args:
    env RUST_BACKTRACE=full cargo run --release --locked {{args}}

# Start a disposable keyring service with synthetic data for manual testing
dev-env *args:
    cargo run --example dev_env -- {{args}}

# Check that Keybox's output and settings contain no synthetic secrets (opens a window)
privacy-check:
    scripts/privacy-check.sh

# Installs files
install:
    install -Dm0755 {{ cargo-target-dir / 'release' / name }} {{bin-dst}}
    install -Dm0644 {{ 'target' / 'xdgen' / 'app.desktop' }} {{desktop-dst}}
    install -Dm0644 {{ 'target' / 'xdgen' / 'app.metainfo.xml' }} {{appdata-dst}}
    install -Dm0644 {{ 'resources' / 'icons' / 'hicolor' / 'scalable' / 'apps' / 'icon.svg' }} {{icon-svg-dst}}

# Uninstalls installed files
uninstall:
    rm {{bin-dst}} {{desktop-dst}} {{appdata-dst}} {{icon-svg-dst}}

# Packaging and releases

Keybox is released the same way as
[cosmic-ext-deskfeed](https://github.com/rabol/cosmic-ext-deskfeed): one tag
builds every package.

## What a release contains

Pushing a tag `v<version>` runs [`.github/workflows/release.yml`](../.github/workflows/release.yml):

| File | Built in | From |
| --- | --- | --- |
| `cosmic-ext-keybox-<version>.tar.gz` | Ubuntu (GitHub runner) | The tag, with every Rust crate in `vendor/`, so it builds offline |
| `cosmic-ext-keybox-<version>-1.fc43.x86_64.rpm` | Fedora 43 | [`rpm/cosmic-ext-keybox.spec`](rpm/cosmic-ext-keybox.spec) |
| `cosmic-ext-keybox_<version>-1_amd64.deb` | Debian testing | [`debian/`](debian) |
| `cosmic-ext-keybox-<version>-1-x86_64.pkg.tar.zst` | Arch Linux | [`aur/cosmic-ext-keybox/PKGBUILD`](aur/cosmic-ext-keybox/PKGBUILD) |
| `SHA256SUMS` | | Checksums of the files above |

Each package is built from the source archive with its distribution's own
Rust, then installed in a fresh container of Fedora 43, Fedora 44, Debian
testing and Arch. There the workflow checks that the installed files are in
place and that every shared library the binary needs came with the package's
dependencies.

On a tag, the workflow then creates a **draft** GitHub release with these
files and the version's section of [`CHANGELOG.md`](../CHANGELOG.md) as
notes. Read it over and publish it by hand.

Run by hand (Actions › Release › Run workflow), the workflow builds and checks
the packages and keeps them as the run's artifacts, without a release. Use
that to try packaging changes.

## Making a release

1. Move the entries under `## [Unreleased]` in `CHANGELOG.md` into what will
   be the new version's section (the next step adds the heading), and add
   release notes to the `<release>` entry in `resources/app.metainfo.xml` if
   you want more than a bare entry.
2. `just tag <version>`. This runs `scripts/set-version.py`, which writes the
   version and today's date into `Cargo.toml`, the rpm spec, the Debian
   changelog, the PKGBUILD, the AppStream metadata and `CHANGELOG.md`. It then
   commits (`release: <version>`) and creates the tag `v<version>`.
3. `git push origin main v<version>`.
4. When the workflow is done, check the draft release and publish it.

`python3 -I scripts/set-version.py --check` prints the version and fails if
the files disagree; the workflow runs it and also fails if the tag doesn't
match.

When dependencies change, run `just licenses` to regenerate
`THIRD-PARTY-LICENSES.md`. It prints the license identifiers compiled into
Keybox, which the spec, the PKGBUILDs and `debian/copyright` list.

## Arch User Repository

The AUR is closed for new accounts (2026-10-27), so the AUR packages are not
published yet. The release workflow builds an Arch package from
`aur/cosmic-ext-keybox/PKGBUILD` for each release instead.

| Package | Builds | Source |
| --- | --- | --- |
| `cosmic-ext-keybox` | A tagged release | GitHub release tarball `v$pkgver` |
| `cosmic-ext-keybox-git` | The latest `main` | `git+https://github.com/rabol/cosmic-ext-keybox.git` |

Both follow the
[Rust package guidelines](https://wiki.archlinux.org/title/Rust_package_guidelines):
dependencies are fetched in `prepare()`, everything is built with
`cargo --frozen`, the tests run in `check()`, and `just install` installs
into the package.

To publish on the AUR once possible:

1. In `aur/cosmic-ext-keybox/`, after the tag is pushed: run `updpkgsums`
   (replaces `SKIP` with the tarball's checksum), `makepkg -f` to test, and
   `makepkg --printsrcinfo > .SRCINFO`.
2. Copy `PKGBUILD` and `.SRCINFO` into a clone of
   `ssh://aur@aur.archlinux.org/cosmic-ext-keybox.git`, commit and push.

The `-git` package's `pkgver` is set by makepkg at build time; push it to the
AUR only when the PKGBUILD itself changes.

# Built by .github/workflows/release.yml in a Fedora container, from the
# source archive with every crate in vendor/, offline. The same spec works
# in Fedora COPR.
#
# Rust's release build has no debug info for find-debuginfo to split off.
%global debug_package %{nil}

Name:           cosmic-ext-keybox
Version:        0.1.0
Release:        1%{?dist}
Summary:        Keyring manager for COSMIC: inspect secrets in your desktop keyring

# Keybox is GPL-3.0-only. The others are the licenses of the crates compiled
# in (THIRD-PARTY-LICENSES.md; `just licenses` lists them). Where a crate
# offers a choice, the permissive one is used.
License:        GPL-3.0-only AND MIT AND Apache-2.0 AND MPL-2.0 AND Unicode-3.0 AND Zlib AND ISC AND BSD-3-Clause AND BSD-2-Clause AND Unlicense AND CC0-1.0
URL:            https://github.com/rabol/cosmic-ext-keybox
Source0:        %{name}-%{version}.tar.gz

ExclusiveArch:  x86_64

# Rust: at least `rust-version` in Cargo.toml.
BuildRequires:  cargo >= 1.93
BuildRequires:  rust >= 1.93
BuildRequires:  gcc
BuildRequires:  just
BuildRequires:  pkgconfig(xkbcommon)
BuildRequires:  pkgconfig(wayland-client)
BuildRequires:  desktop-file-utils

# Keybox is a client: it needs a Secret Service provider in the session,
# but does not pick one.
Recommends:     (oo7-daemon or gnome-keyring)
Recommends:     cosmic-icon-theme
# Opens the project page from About.
Recommends:     xdg-utils

%description
Keybox shows which application credentials are stored in your desktop
keyring and lets you inspect, reveal and copy a selected entry, create and
edit text entries, delete entries, and manage keyrings. It works with any
freedesktop Secret Service provider, such as oo7 or GNOME Keyring. Made for
the COSMIC desktop; not affiliated with System76.

%prep
%autosetup -n %{name}-%{version}

%build
cargo build --frozen --release

%install
just rootdir=%{buildroot} prefix=%{_prefix} install

%check
desktop-file-validate %{buildroot}%{_datadir}/applications/dev.rabol.Keybox.desktop

%files
%license LICENSE THIRD-PARTY-LICENSES.md
%doc README.md CHANGELOG.md docs/user-guide.md
%{_bindir}/%{name}
%{_datadir}/applications/dev.rabol.Keybox.desktop
%{_datadir}/metainfo/dev.rabol.Keybox.metainfo.xml
%{_datadir}/icons/hicolor/scalable/apps/dev.rabol.Keybox.svg

%changelog
* Tue Oct 27 2026 Steen Rabol <steen@rabol.dev> - 0.1.0-1
- First package; see CHANGELOG.md

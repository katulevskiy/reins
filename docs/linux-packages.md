# Linux packages

Every release of the desktop app is also a set of Linux packages:

| Package | What | Architectures |
| --- | --- | --- |
| `reins` | the command-line program, `/usr/bin/reins` (the same static build as the release archives) | x86_64 (amd64), aarch64 (arm64) |
| `reins-app` | the Reins app (window and tray), `/usr/bin/reins-app`, with its menu entry; needs `reins` | x86_64 (amd64) |
| `reins-bin` (AUR) | both of the above in one Arch Linux package | x86_64 (aarch64: `reins` only) |

They come from signed package repositories at `https://reins2fa.com/releases/packages/`, which always hold the latest
release, so the system's updates bring new versions. The same `.deb` and `.rpm` files are assets of every
[GitHub release](https://github.com/katulevskiy/reins/releases/latest), for installing one by hand.

`reins` runs on any distribution; `reins-app` needs glibc 2.35 or later (Ubuntu 22.04, Debian 12, Fedora 36 and later)
and a Vulkan driver (`mesa-vulkan-drivers`, or your GPU vendor's), which the packages recommend.

## Debian and Ubuntu

```sh
sudo install -d -m 0755 /etc/apt/keyrings
curl -fsSL https://reins2fa.com/releases/packages/reins.gpg | sudo tee /etc/apt/keyrings/reins.gpg > /dev/null
sudo tee /etc/apt/sources.list.d/reins.sources > /dev/null <<'EOF'
Types: deb
URIs: https://reins2fa.com/releases/packages/apt
Suites: stable
Components: main
Signed-By: /etc/apt/keyrings/reins.gpg
EOF
sudo apt update
sudo apt install reins           # and reins-app for the desktop app
```

The key file trusts this key for this repository only (`Signed-By`), not for the rest of the system. On apt older than
1.1, which does not read `.sources` files, use the one-line form instead:

```sh
echo "deb [signed-by=/etc/apt/keyrings/reins.gpg] https://reins2fa.com/releases/packages/apt stable main" |
    sudo tee /etc/apt/sources.list.d/reins.list
```

A downloaded package installs the same way, without the repository: `sudo apt install ./reins_<version>_amd64.deb`.

## Fedora, RHEL and derivatives

```sh
sudo curl -fsSL https://reins2fa.com/releases/packages/rpm/reins.repo -o /etc/yum.repos.d/reins.repo
sudo dnf install reins           # and reins-app for the desktop app
```

The first install asks to import the repository key and shows its fingerprint. `reins.repo` is:

```ini
[reins]
name=Reins
baseurl=https://reins2fa.com/releases/packages/rpm
enabled=1
gpgcheck=1
repo_gpgcheck=1
gpgkey=https://reins2fa.com/releases/packages/reins.asc
```

`gpgcheck` checks each package's signature and `repo_gpgcheck` the signature of the repository's metadata, both with
that key. A downloaded package: `sudo dnf install ./reins-<version>-1.x86_64.rpm`.

## Arch Linux

`reins-bin` is on the [AUR](https://aur.archlinux.org/packages/reins-bin). With an AUR helper, `yay -S reins-bin` or
`paru -S reins-bin`; by hand:

```sh
git clone https://aur.archlinux.org/reins-bin.git
cd reins-bin
makepkg -si
```

Its PKGBUILD downloads the release's archives from GitHub and checks them against the SHA-256 published with the
release.

## Checking the key

The key the repositories are signed with is also in the source repository, as
`scripts/package/linux-packages/packages-key.asc` ("Reins packages <support@reins2fa.com>", fingerprint
`134A 8E7B 8571 0C27 D81C  A79F 5620 15A5 8DF5 6B22`). Compare its fingerprint with the one you installed:

```sh
gpg --show-keys /etc/apt/keyrings/reins.gpg                    # Debian, Ubuntu
gpg --show-keys <(curl -fsSL https://reins2fa.com/releases/packages/reins.asc)   # Fedora
```

## After installing

Continue with `reins login` and `reins resume` ([quick start](quick-start.md#3b-install-the-desktop-app)), or open
**Reins** from the applications menu. Updates come with the system's (`sudo apt upgrade`, `sudo dnf upgrade`, your AUR
helper); `reins update` only says that, as it does not replace a program the package manager owns. After an upgrade
the packages restart the background service of every logged-in user who runs it, so it runs the new program.

If you used the install script before, remove its copy, `~/.local/bin/reins` (and `~/.local/bin/Reins.AppImage` with
`~/.local/share/applications/reins.desktop` for the app), then run `reins resume` again: `~/.local/bin` usually comes
first on the `PATH`, and the background service runs the program that set it up.

Before removing the packages, run `reins pause` and `reins service uninstall` as each user who set Reins up: they
send git straight to the git hosts again and remove the background service, which the package removal leaves in
place (it is the user's, in their home).

## Where things are

| URL | What |
| --- | --- |
| `https://reins2fa.com/releases/packages/reins.gpg`, `reins.asc` | the repositories' public key (binary, armored) |
| `https://reins2fa.com/releases/packages/apt` | the APT repository: suite `stable`, component `main`, `amd64` and `arm64`; `dists/stable/InRelease` (and `Release`, `Release.gpg`) signed |
| `https://reins2fa.com/releases/packages/rpm` | the RPM repository (both architectures): `repodata/repomd.xml` with `repomd.xml.asc`, packages signed |
| `https://reins2fa.com/releases/packages/rpm/reins.repo` | the repository definition for dnf |

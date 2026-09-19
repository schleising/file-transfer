# File Transfer

macOS app that orchestrates **direct** file transfers between computers using **SSH** and **Homebrew rsync**. See [docs/DESIGN.md](docs/DESIGN.md). Android on the LAN is assessed in [docs/ANDROID.md](docs/ANDROID.md) (not implemented).

## Build & install (personal use)

```bash
# Prerequisites: Xcode CLT, Rust (rustup), Homebrew rsync
brew install rsync
./scripts/install-app.sh
```

This builds **File Transfer.app** and copies it to `/Applications`.

On first launch after install, macOS asks to allow **local network** access. Allow it, or enable **File Transfer** under **System Settings → Privacy & Security → Local Network**. Without that grant, SSH to LAN hosts fails with **no route to host** and Bonjour `_ssh._tcp` discovery stays empty. If the toggle is already on after an OS update, turn it off and on again.

Data is stored under `~/Library/Application Support/File Transfer/`.

## Develop

```bash
cargo run -p ft-app
```

Supported run mode for daily use is still the `/Applications` app from `install-app.sh`.

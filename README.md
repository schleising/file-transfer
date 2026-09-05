# File Transfer

macOS app that orchestrates **direct** file transfers between computers using **SSH** and **Homebrew rsync**. See [docs/DESIGN.md](docs/DESIGN.md).

## Build & install (personal use)

```bash
# Prerequisites: Xcode CLT, Rust (rustup), Homebrew rsync
brew install rsync
./scripts/install-app.sh
```

This builds **File Transfer.app** and copies it to `/Applications`.

Data is stored under `~/Library/Application Support/File Transfer/`.

## Develop

```bash
cargo run -p ft-app
```

Supported run mode for daily use is still the `/Applications` app from `install-app.sh`.

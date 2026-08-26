# Third-party notices

This project is distributed as source, as a browser bundle, and as a container
image. Each of those carries third-party code whose licences require their
copyright notices to travel with it. Those notices are reproduced here.

Nothing in this file grants rights over this project itself — see `LICENSE`.

---

## 1. `@cloudflare/speedtest`

The measurement engine. Pinned exactly in `frontend/package.json`, bundled into
the front-end JavaScript, and therefore present in the container image and in
anything served from it.

- Upstream: https://github.com/cloudflare/speedtest
- Licence: MIT

```
MIT License

Copyright (c) 2023 Cloudflare

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.
```

Cloudflare is a trademark of Cloudflare, Inc. This project is not affiliated
with, sponsored by, or endorsed by Cloudflare, Inc. The engine is used under
the MIT licence above; the name is used only to identify it.

---

## 2. Rust dependencies (statically linked into the server binary)

The backend links its entire dependency tree statically, so the binary shipped
in the container image contains code from every crate in `backend/Cargo.lock`.
That set is licensed under permissive terms throughout — MIT, Apache-2.0, ISC,
BSD, and Unicode-3.0. **No crate in the tree is under the GPL, LGPL or AGPL.**

`backend/Cargo.lock` is the authoritative, versioned list. To regenerate the
per-crate licence enumeration (from WSL, where the toolchain lives):

```sh
cargo install cargo-about
cargo about generate --manifest-path backend/Cargo.toml about.hbs > THIRD-PARTY-RUST.html
```

Three components in that tree warrant naming individually:

**SQLite**, vendored via `libsqlite3-sys` with the `bundled` feature. SQLite is
released into the public domain by its authors and imposes no conditions.
https://sqlite.org/copyright.html

**`ring`** and **`untrusted`**, which provide the cryptography behind `rustls`.
`ring` carries an ISC-style licence over its own code and retains the original
notices of the BoringSSL and OpenSSL-derived code it incorporates. Both notices
travel in the crate source. https://github.com/briansmith/ring/blob/main/LICENSE

**ICU4X** (`icu_*`, `zerovec`, `yoke`, `tinystr` and related), under the
Unicode License v3. https://www.unicode.org/license.txt

---

## 3. Container base image

The runtime image derives from `debian:bookworm-slim` and installs `curl` and
`libcap2-bin` from the Debian archive. Publishing that image redistributes
Debian binaries, some of which are under the GPL.

Debian publishes the complete corresponding source for every binary package it
ships, and that source is the offer relied on here:

- https://www.debian.org/distrib/packages
- `deb-src http://deb.debian.org/debian bookworm main`

The exact package set in any given image is recoverable from the image itself
with `dpkg-query -W -f='${Package} ${Version} ${Homepage}\n'`.

The build stages (`node:22-bookworm-slim`, `rust:1.90-bookworm`) are not
redistributed; they produce artifacts and are discarded.

---

## 4. Not redistributed by this project

**coturn** provides the TURN relay for the packet-loss stage. It is installed
on the guest from the Debian archive by `provisioning/coturn/install-coturn.sh`
and is never bundled, vendored, or shipped in the image. This project ships a
configuration template only. coturn is licensed under the 3-clause BSD licence.

**Front-end build and test tooling** — Vite, Vitest, TypeScript, Playwright —
are development dependencies. They are used to produce the build and do not
appear in it.

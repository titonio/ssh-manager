# Cross-compiling `aarch64-unknown-linux-musl` on GitHub Actions — research findings

Researched 2026-09-20 against primary sources only (tool READMEs/source, ziglang.org,
cross-rs docs, GitHub docs/changelog, musl upstream, vendor pages). Repo context:
ratatui/crossterm/serde/clap + self-update crate (rustls + ring 0.17.14 +
quinn-proto); flate2 = pure-Rust miniz_oxide; zip 0.6.6 pure Rust; **no**
openssl-sys / aws-lc-rs / libz-ng / zstd / lzma. CI today: `ubuntu-latest` +
Swatinem/rust-cache + actions-rs/toolchain + taiki-e/install-action, recipe =
cargo-zigbuild + Zig 0.16.0 + qemu-user-static verification.

**Key structural fact:** the only crate in the graph that compiles C/asm is
**ring** (pregenerated assembly driven by `cc-rs`). Everything else is pure Rust.
That makes the choice of cross toolchain mostly a question of "who provides a
correct aarch64-musl C/asm toolchain + libc, cheaply and reliably", not of
handling a complex C dependency tree.

---

## 1. cargo-zigbuild

Source: [README](https://github.com/rust-cross/cargo-zigbuild) ·
[wrapper source `src/zig/mod.rs`](https://github.com/rust-cross/cargo-zigbuild/blob/main/src/zig/mod.rs) ·
[build wiring `src/build.rs`](https://github.com/rust-cross/cargo-zigbuild/blob/main/src/build.rs)

### How it works
- Runs `cargo build` with **`zig cc` as the linker** (`CARGO_TARGET_<T>_LINKER`
  pointed at a generated wrapper) and zig wrappers for the C toolchain that
  build scripts invoke. The `cargo-zigbuild` binary itself exposes wrapper
  subcommands `cc`, `c++`, `ar`, `ranlib`, `lib`, `dlltool`
  ([`src/zig/mod.rs`](https://github.com/rust-cross/cargo-zigbuild/blob/main/src/zig/mod.rs)),
  which `cc-rs`/`cmake-rs` etc. are pointed at via `CC`/`CXX`/`AR`/`RANLIB`
  env. The wrapper rewrites/normalizes rustc linker args before passing them to
  `zig cc` (response-file `@…linker-arguments` handling, arg filtering,
  `-target` dedup).
- It also patches known gaps itself: e.g. a **musl weak-symbols map**
  (`musl_weak_symbols_map.ld`) and ARM `__aeabi_uread/write` shim compiled into
  the link for strict-align ARM targets
  ([`src/zig/mod.rs`](https://github.com/rust-cross/cargo-zigbuild/blob/main/src/zig/mod.rs)).
- Exports `CARGO_ZIGBUILD_TARGET` (e.g. `x86_64-linux-gnu.2.36`) to build
  scripts, plus `BINDGEN_EXTRA_CLANG_ARGS[_<target>]`, `CMAKE_TOOLCHAIN_FILE`,
  `SDKROOT` (README env table).
- Caveat (README): "Currently only Linux and macOS targets are supported"; only
  current Rust stable/nightly regularly tested.

### musl specifics — what Zig 0.16 bundles
- **Zig 0.16.0 ships musl 1.2.5 plus backported security fixes** (upstream has
  tagged 1.2.6; a future Zig will move to it). When targeting musl statically,
  many functions now come from **zig libc** rather than copied musl sources:
  "331 fewer musl C source files are now distributed with Zig, with 1,206
  remaining" — bugs go to Zig's tracker, not musl's. Zig 0.16.0 "is not
  believed to be affected by CVE-2026-40200 due to musl's `qsort`/`qsort_r`
  no longer being used."
  ([Zig 0.16.0 release notes, "musl 1.2.5"](https://ziglang.org/download/0.16.0/release-notes.html))
- `aarch64-linux` is **Tier 2** in Zig 0.16 with libc ✅ and CI ✅
  ([release notes support table](https://ziglang.org/download/0.16.0/release-notes.html)).
  Zig itself requires Linux ≥ 5.10 to run (fine on `ubuntu-latest`).
- musl upstream context: latest release is **1.2.6**; advisories: CVE-2026-40200
  (all releases through 1.2.6, 32-bit archs), CVE-2026-6042 (iconv DoS,
  through 1.2.6), CVE-2025-26519 (through 1.2.5)
  ([musl.libc.org](https://musl.libc.org/)). Whether Zig's "backported security
  fixes" cover each of these individually is **not enumerated** in the release
  notes (only CVE-2026-40200 is explicitly addressed).

### Static-linking guarantee
- README is explicit: `-C target-feature=+crt-static` against **glibc** is *not*
  supported ("upstream `zig cc` lacks support") — **"Use a `*-musl` target
  instead if you need a fully static binary."** For musl, static linking is the
  normal/default mode of the Rust target itself (rustc's musl targets default to
  static libc; the rustc book says the target "is distributed through `rustup`,
  and otherwise requires no special configuration" —
  [rustc book: aarch64-unknown-linux-musl](https://doc.rust-lang.org/rustc/platform-support/aarch64-unknown-linux-musl.html)).
  Caveat: `RUSTFLAGS` like `-C linker` opt out of zig; `-L path` "will have Zig
  ignore `-C target-feature=+crt-static`" (README glibc caveats list).

### Known pitfalls (README-documented)
- **bindgen + Zig ≥ 0.15**: "you may need clang 18+ installed for bindgen to
  work correctly. This is because zig 0.15+ bundles libc++ 19 headers which
  require a compatible libclang version." → **Not applicable to this repo**
  (no bindgen in the dependency set).
- **`-nostdinc` behavior**: `cargo zigbuild` always passes `-nostdinc`, which
  excludes standard header locations like `/usr/include` (and Zig's `--target`
  mode opts out of system search paths generally). Consequence: headers/libs
  installed via apt are invisible unless you add `CFLAGS='-isystem /usr/include'`
  / `RUSTFLAGS='-L /usr/lib64'`. Mixing system glibc headers with Zig's own
  causes `__GLIBC_MINOR__` redefinition errors — the README shows why `-isystem`
  (not `CPATH`) is the right escape hatch. For a musl target with only ring as a
  C/asm consumer, this rarely bites (Zig provides its own musl headers).
- **glibc version suffix is `*-gnu` only**: the `--target …-gnu.2.17` suffix
  feature is documented exclusively for glibc targets (default glibc varies per
  Zig release, e.g. [Target.zig @ 0.14.1](https://github.com/ziglang/zig/blob/0.14.1/lib/std/Target.zig#L473)).
  There is **no version suffix for musl** — musl has no versioned-symbol ABI to
  target, so nothing analogous exists in the README.
- **target-cpu passthrough**: README documents that `cargo zigbuild` picks up
  `-C target-cpu` from `RUSTFLAGS` and passes a matching `-mcpu` to `zig cc` so
  C/C++ deps get the same CPU; also works via `--config` / `.cargo/config.toml`
  aliases. Upstream caveat: Zig issue
  [#10411 "CPU features are not passed to clang"](https://github.com/ziglang/zig/issues/10411)
  and [#4911 `-target`/`-mcpu` parsing](https://github.com/ziglang/zig/issues/4911)
  (armv7 gnueabihf needs `-mcpu=generic` workaround,
  [zigbuild PR #58](https://github.com/rust-cross/cargo-zigbuild/pull/58));
  compiler-rt gaps ([zig #1290](https://github.com/ziglang/zig/issues/1290)).
  For a plain `aarch64-unknown-linux-musl` baseline build none of this is
  load-bearing.

### ring + zig cross-build: known-good?
- **Yes, with a version constraint.** cargo-zigbuild's own CI test suite includes
  `tests/hello-rustls` and `tests/hello-tls` with ring (Dependabot bumps
  [PR #323](https://github.com/rust-cross/cargo-zigbuild/pull/323),
  [PR #326](https://github.com/rust-cross/cargo-zigbuild/pull/326) bumped ring
  0.17.8→0.17.13 in those tests) — ring+rustls cross-builds are continuously
  exercised upstream.
- **Real bug, fixed**: [issue #433 "zig 0.16.0 failed to build ring"](https://github.com/rust-cross/cargo-zigbuild/issues/433)
  — with **zig 0.16.0 + cargo-zigbuild 0.22.2**, the `ar` wrapper failed
  (`ring@0.16.20: ar: error: unable to open … libring-core.a`); root cause was
  upstream Zig ([codeberg zig#32040](https://codeberg.org/ziglang/zig/issues/32040));
  confirmed resolved with **cargo-zigbuild 0.23.0** + zig 0.16.0 (reporter
  follow-up 2026-07-05). → **Pin `cargo-zigbuild ≥ 0.23.0` when on Zig 0.16.**
- Older closed issues: [#253](https://github.com/rust-cross/cargo-zigbuild/issues/253)
  (ring 0.17.8 SIGILL, armv7), [#217](https://github.com/rust-cross/cargo-zigbuild/issues/217)
  (CC-variable crates incl. ring), [#132](https://github.com/rust-cross/cargo-zigbuild/issues/132)
  (ring 0.16.20). All closed; none specific to ring 0.17.14 + aarch64-musl.
  I found **no open issue** matching ring 0.17.14 + zig 0.16 + aarch64-musl.

### Termux/Android mention
- GitHub issue search for `termux` in rust-cross/cargo-zigbuild: **0 results**.
  The README never mentions Termux. Android appears only tangentially
  ([#300 "Is it suitable to use zigbuild to build the crate of ring for
  Android?"](https://github.com/rust-cross/cargo-zigbuild/issues/300), closed).
  Note the conceptual mismatch anyway: Termux is Android/**bionic**, not musl —
  there is no Termux-musl target story in zigbuild's docs.

### CI fit
- `taiki-e/install-action` installs `cargo-zigbuild` from GitHub Releases with
  SHA256 + attestation verification
  ([TOOLS.md](https://github.com/taiki-e/install-action/blob/main/TOOLS.md)).
  Zig itself: download from ziglang.org (0.16.0 x86_64-linux tarball =
  **55,478,392 bytes ≈ 53 MiB**, verified via `Content-Length`) or `pip install
  ziglang` (README).
- rustc/cargo run natively on the runner; only C/asm goes through `zig cc`.
  Swatinem/rust-cache works unchanged (build happens on the host).

---

## 2. cross (cross-rs)

Source: [README](https://github.com/cross-rs/cross) ·
[docs/environment_variables.md](https://github.com/cross-rs/cross/blob/main/docs/environment_variables.md) ·
[docs/getting-started.md](https://github.com/cross-rs/cross/blob/main/docs/getting-started.md) ·
[docs/recipes.md](https://github.com/cross-rs/cross/blob/main/docs/recipes.md) ·
[wiki FAQ](https://github.com/cross-rs/cross/wiki/FAQ)

### How it works
- "Zero setup" cross compilation: `cross build --target X` runs the build inside
  a **prebuilt Docker image per target** published at
  `ghcr.io/cross-rs/<target>` (e.g. `ghcr.io/cross-rs/aarch64-unknown-linux-musl`).
  The image ships the cross GCC toolchain + prebuilt target libs.
- Per the README's target table, `aarch64-unknown-linux-musl` = **musl 1.2.3,
  GCC 9.2.0, QEMU 6.1.0, `cross test` ✓**. (Note: musl 1.2.3 is from 2021 —
  older than the 1.2.4/1.2.5/1.2.6 security fixes listed on
  [musl.libc.org](https://musl.libc.org/); cross images are updated on their
  own cadence, and you can pin by `@sha256:` digest per
  [issue #1639 discussion](https://github.com/cross-rs/cross/issues/1639).)
- **Image size (measured via ghcr registry API, `:main` tag, 2026-09-20):**
  7 layers, **≈ 437 MB compressed** for `cross-rs/aarch64-unknown-linux-musl`.

### Runner requirements
- Container engine: **Docker ≥ 20.10 (API 1.40) or Podman ≥ 3.4** (README
  Dependencies). GitHub-hosted Ubuntu runners ship Docker: the standard
  `ubuntu-24.04` image inventory lists **Docker Server 28.0.4** + client +
  Buildx + Compose
  ([runner-images Ubuntu2404 inventory](https://github.com/actions/runner-images/blob/main/images/ubuntu/Ubuntu2404-Readme.md)),
  and the arm64 partner image inventory likewise lists Docker/Compose/Buildx
  and Podman
  ([partner-runner-images arm-ubuntu-24](https://github.com/actions/partner-runner-images/blob/main/images/arm-ubuntu-24-image.md)).
  So cross runs on both `ubuntu-latest` and `ubuntu-24.04-arm` without extra
  setup.
- **binfmt_misc** kernel support is required for *cross testing* outside the
  container; the FAQ notes registering it (`docker run --privileged
  tonistiigi/binfmt --install all`) "requires `--privileged` because it
  modifies the host kernel"
  ([FAQ](https://github.com/cross-rs/cross/wiki/FAQ)). `cross run/test` itself
  executes foreign binaries via an in-container QEMU runner (`/linux-runner`),
  so builds/tests on GitHub runners don't need privileged containers. I found
  **no blanket privileged requirement** in cross's docs for normal
  build/test/test workflows.
- Docker-in-Docker supported via `CROSS_CONTAINER_IN_CONTAINER=true` (README).

### Caching story
- cross passes through `CARGO_HOME` (and `CARGO_TARGET_DIR`, `RUSTFLAGS`,
  `CARGO_*`/`CROSS_*` generally) into the container
  ([environment_variables.md](https://github.com/cross-rs/cross/blob/main/docs/environment_variables.md)),
  and the host `CARGO_HOME`/workspace are bind-mounted into the container (the
  Lima FAQ section explicitly treats `~/.cargo` = `CARGO_HOME` as a mounted
  volume, [FAQ](https://github.com/cross-rs/cross/wiki/FAQ)). Practical
  consequence: **host-side caching of `~/.cargo` and `./target` (i.e.
  Swatinem/rust-cache) remains effective under cross**, because the container
  reads/writes those same host paths. Caveat: I found **no explicit
  cross-docs statement naming rust-cache**; this is inferred from the mount +
  passthrough documentation. File ownership can differ (container runs as
  uid/gid; `CROSS_CONTAINER_UID/GID` exist) — rust-cache restore/save happens
  host-side so this usually isn't an issue, but it's the classic friction point.
- For C-compilation caching specifically, cross's documented route is
  **sccache inside a custom image** with env passthrough
  ([recipes.md](https://github.com/cross-rs/cross/blob/main/docs/recipes.md)).

### Known limitations
- `cross test` is **slow and sequential**: "runs unit tests *sequentially*
  because QEMU gets upset when you spawn multiple threads" (README).
- Test failures can be QEMU bugs, not yours (README).
- MSRV: cross itself compiles on Rust ≥ 1.85 (README).
- Image freshness: musl 1.2.3 baseline (see above); ring needs binutils ≥ 2.30
  for x86_64 VPCLMULQDQ — [cross #1639](https://github.com/cross-rs/cross/issues/1639)
  was an **illumos-only** manifestation (binutils 2.28 in that image);
  aarch64-musl images (binutils from GCC 9.2 era) are unaffected.
  [cross #985 "Missing intrinsic functions on aarch64/armv7 musl"](https://github.com/cross-rs/cross/issues/985)
  is closed.

### What it would change for this repo
- Replace `cargo zigbuild` with `cross build --target aarch64-unknown-linux-musl`
  + `cross test` (gives you cross-*testing* under QEMU for free, vs. the
  current qemu-user-static verification step).
- Costs: ~437 MB image pull per runner (uncached), container mount overhead,
  and you lose the "everything runs natively on the host" simplicity. The
  musl in the image (1.2.3) is **older** than Zig 0.16's 1.2.5+backports.
- No benefit from the repo's perspective over zigbuild except cross test
  ergonomics and not depending on Zig's libc at all.

---

## 3. Native ARM runners on GitHub Actions

### Availability & labels
- Labels: **`ubuntu-24.04-arm`, `ubuntu-22.04-arm`** (and now `ubuntu-26.04-arm`)
  — [GitHub docs: GitHub-hosted runners reference](https://docs.github.com/en/actions/reference/runners/github-hosted-runners).
- Timeline (all GitHub primary):
  - **2024-06-03** — arm64 hosted runners public beta, Team/Enterprise plans,
    "priced 37% less than x64"
    ([changelog](https://github.blog/changelog/2024-06-03-actions-arm-based-linux-and-windows-runners-are-now-in-public-beta/)).
  - **2025-01-16** — **free for public repositories** (Public Preview),
    Cobalt 100-based, 4 vCPU, "up to 40% performance boost" vs Azure's prior
    arm64 VMs; labels "will not work in private repositories"; standard usage
    limits/concurrency per plan; possible queue times in peak
    ([changelog](https://github.blog/changelog/2025-01-16-linux-arm64-hosted-runners-now-available-for-free-in-public-repositories-public-preview/)).
  - **2026-01-29** — arm64 **standard runners now available in private
    repos**: 2 vCPU private / 4 vCPU public, counts toward the plan's free
    minutes, "fully supported standard GitHub-hosted runners"
    ([changelog](https://github.blog/changelog/2026-01-29-arm64-standard-runners-are-now-available-in-private-repositories/)).
- Hardware per docs table: public repos **4 vCPU / 16 GB RAM / 14 GB SSD
  (arm64)**; private repos **2 vCPU / 8 GB RAM / 14 GB SSD (arm64)**
  ([docs](https://docs.github.com/en/actions/reference/runners/github-hosted-runners)).
- Images: originally Arm-maintained "partner images"
  ([actions/partner-runner-images](https://github.com/actions/partner-runner-images));
  GitHub has since taken ownership — that repo was **archived June 12 2026**,
  image management moved to
  [actions/runner-images](https://github.com/actions/runner-images). The arm64
  Ubuntu image inventory lists Docker, Docker Compose, Buildx, and Podman
  preinstalled
  ([arm-ubuntu-24 inventory](https://github.com/actions/partner-runner-images/blob/main/images/arm-ubuntu-24-image.md)).

### Does native ARM remove the cross toolchain?
Mostly yes — but you still need a musl libc for aarch64:
- `rustup target add aarch64-unknown-linux-musl` installs rust-std for the
  target; the rustc book says the target "is distributed through `rustup`, and
  otherwise requires no special configuration" and "can be cross-compiled from
  any host"
  ([rustc book](https://doc.rust-lang.org/rustc/platform-support/aarch64-unknown-linux-musl.html)).
  rust-std for musl targets ships the static musl libc/crt objects in the
  target's `self-contained` dir (community-confirmed: "the libc.a shipped by
  rustup" — [tweag blog](https://www.tweag.io/blog/2023-08-10-rust-static-link-with-mimalloc);
  [users.rust-lang.org thread](https://users.rust-lang.org/t/installation-of-rust-with-x86-64-unknown-linux-musl-target/42922)
  shows a musl binary linking with no musl-gcc at all). **I could not dump the
  rust-std manifest from this sandbox to verify byte-for-byte** — treat the
  self-contained-libc claim as community-corroborated, with the rustc book as
  the official anchor.
- **`musl-tools` on Ubuntu arm64: yes, it exists.** noble ships
  `musl-tools 1.2.4-2` (universe) for **arm64** (and amd64/armhf), providing
  `/usr/bin/musl-gcc` and `/usr/bin/musl-ldd`
  ([packages.ubuntu.com noble musl-tools filelist](https://packages.ubuntu.com/noble/arm64/musl-tools/filelist)).
  The package is arch-specific, so on an arm64 runner `musl-gcc` targets
  aarch64-linux-musl natively. (Ubuntu's musl-tools builds musl-gcc for the
  native arch — there is no cross-from-x86_64 use; that's exactly the native-ARM
  case.)
- **ring on native aarch64**: ring's build compiles pregenerated asm via
  `cc-rs`; on a native aarch64 host the system C compiler handles this without
  any cross toolchain. ring's README warns to be wary of toolchains/targets
  other projects don't use, but aarch64-linux-musl is a mainstream supported
  target
  ([ring README](https://github.com/briansmith/ring/blob/main/README.md)).
- Net: on `ubuntu-24.04-arm`, `cargo build --target aarch64-unknown-linux-musl`
  needs **no zig, no cross, no qemu** — and crucially, **verification runs
  natively** (you can execute the produced binary in CI instead of
  qemu-user-static). The remaining choice is which libc the C bits compile
  against: rust-std's bundled musl (default), or `musl-tools`' musl-gcc
  (1.2.4) if you want an explicit musl C compiler.

### Self-hosted ARM options (brief)
- **Oracle Cloud Always Free — Ampere A1 (`VM.Standard.A1.Flex`)**: all
  tenancies get **1,500 OCPU-hours + 9,000 GB-hours/month free** (≈ 2 OCPU +
  12 GB always-on; flexible), 200 GB block volume; idle instances may be
  reclaimed (<20% CPU/net/mem over 7 days); "out of capacity" errors are
  common in popular regions
  ([Oracle Always Free docs](https://docs.oracle.com/en-us/iaas/Content/FreeTier/freetier_topic-Always_Free_Resources.htm)).
  Runner registration: [Adding self-hosted runners](https://docs.github.com/en/actions/hosting-your-own-runners/managing-self-hosted-runners/adding-self-hosted-runners).
- **Hetzner CAX (ARM64, Ampere)**: CAX11 2 vCPU / 4 GB / 40 GB NVMe / 20 TB
  traffic ≈ **€5.99/mo** (pre-VAT figures lower; verify on the official page)
  ([hetzner.com/cloud](https://www.hetzner.com/cloud/),
  [aggregator snapshot](https://costgoat.com/pricing/hetzner)).
- **Scaleway**: legacy Stardust was €0.0025/hr (~€2/mo, limited supply —
  [TechCrunch 2020](https://techcrunch.com/2020-11-02/scaleway-launches-cloud-instances-that-cost-2-10-per-month/));
  current ARM/prosumer lines are on the
  [Scaleway pricing page](https://www.scaleway.com/en/pricing/virtual-instances);
  GitHub-runner-on-Scaleway tutorial:
  [scaleway.com/docs/tutorials/host-github-runner](https://www.scaleway.com/en/docs/tutorials/host-github-runner/).
  (I could not extract a current ARM instance price from Scaleway's JS-driven
  pages — **unverified**.)
- Self-hosted runner registration docs:
  [docs.github.com — adding self-hosted runners](https://docs.github.com/en/actions/hosting-your-own-runners/managing-self-hosted-runners/adding-self-hosted-runners).

---

## 4. Hand-rolled musl cross toolchain

### musl-cross-make (build from source)
- [richfelker/musl-cross-make](https://github.com/richfelker/musl-cross-make)
  README: makefile-based builder for "musl-targeting cross compilers"; single-stage
  GCC build; downloads GCC/binutils/GMP/MPC/MPFR + musl sources with hash checks;
  `make && make install`; `TARGET=aarch64-linux-musl` is in the supported list.
  License: tools are MIT/Expat; **patches and resulting binaries retain upstream
  (GPL etc.) licenses** — the README is explicit that the MIT license "does not
  cover the patches or resulting binary artifacts."
- **Build time: not stated in the README.** It's a full GCC+binutils+musl build;
  budget tens of minutes to hours per cold build depending on runner size
  (estimate — no primary figure). Caching the built toolchain is mandatory for
  CI sanity.

### musl.cc prebuilts
- [musl.cc](https://musl.cc/) hosts `aarch64-linux-musl-cross.tgz` —
  **108,096,828 bytes (~103 MiB)**, dated **2021-11-23** (directory listing).
  Ingredients: **musl git-b76f37f (2021-09-23), GCC 11.2.1, binutils 2.37,
  Linux 5.15.2 headers** — i.e. a 2021-vintage libc, predating CVE-2025-26519
  and everything newer on [musl.libc.org](https://musl.libc.org/).
- **Reliability red flag (site's own words): "2025-05-27: GitHub Actions has
  been blocked wholesale due to abuse"** — direct downloads from GH-hosted
  runners are blocked. Also: "old binaries might not be archived", "versions may
  be bumped without notice", community site "not officially endorsed by musl".
  Signed with key `0xB1D0B4566FBBDB40` + SHA512SUMS provided.
- **Conclusion: unusable directly from GitHub-hosted runners** without a proxy /
  vendored mirror. Not a viable CI path here.

### Wiring (any external toolchain)
- Naming convention: the toolchain binaries are prefixed
  `aarch64-linux-musl-` (`aarch64-linux-musl-gcc`, `-g++`, `-ar`, …) — the
  same prefix rustc's own bootstrap config uses
  ([rustc book target page](https://doc.rust-lang.org/rustc/platform-support/aarch64-unknown-linux-musl.html)
  shows `cc = "aarch64-linux-musl-gcc"`, `linker = "aarch64-linux-musl-gcc"`).
- For cargo: `CC_aarch64_unknown_linux_musl=aarch64-linux-musl-gcc` (and
  `CXX_…`) for build scripts, and
  `CARGO_TARGET_AARCH64_UNKNOWN_LINUX_MUSL_LINKER=aarch64-linux-musl-gcc` for
  linking. (Env-var names per cargo config; the rustc book page confirms the
  linker/cc variable shape for this target.)

---

## 5. Per-option CI cost & fit (vs current stack)

Download sizes verified where marked ✓; wall-clock build times are **estimates**
(no primary source exists for this repo's graph — label as such).

| Option | Added download | Build wall-clock (est.) | Cache compatibility | Fit notes |
|---|---|---|---|---|
| **cargo-zigbuild + Zig 0.16** (current) | Zig tarball **53 MiB ✓**; zigbuild binary via install-action (small ✓) | rustc native speed; ring asm via zig cc adds little; cold full build of this graph ~3–8 min on 4-core | **Swatinem/rust-cache works as-is** (host build) | Keep, but pin **cargo-zigbuild ≥ 0.23.0** (zig 0.16 `ar` bug, [#433](https://github.com/rust-cross/cargo-zigbuild/issues/433)). musl 1.2.5+backports ✓. Verification still needs qemu-user. |
| **cross** | image pull **≈437 MB compressed ✓** (uncached; ~15–60 s on GH runners, est.) | same rustc work inside container + mount overhead; `cross test` sequential/slow (README) | rust-cache effective via mounted `CARGO_HOME`/target (inferred from docs, not explicitly stated); sccache route documented for C | Adds cross-*testing* under QEMU; older musl (1.2.3); Docker dependency. Marginal gain over current recipe for this repo. |
| **Native `ubuntu-24.04-arm`** | none beyond rustup target (small) | native compile, no emulation; **verification runs natively — no qemu** | rust-cache works as-is (host build) | Free on public repos (4 vCPU/16 GB ✓); 2 vCPU/8 GB on private (counts vs plan minutes ✓). Needs `musl-tools` (arm64 ✓, musl 1.2.4) only if you want an explicit musl gcc; pure-Rust + ring work off rust-std's bundled musl. **Removes zig + qemu entirely.** |
| **musl.cc prebuilt** | 103 MiB ✓ | n/a | n/a | **GitHub Actions IPs blocked wholesale (site statement, 2025-05-27)** → not viable without mirror. 2021 musl. |
| **musl-cross-make from source** | source downloads + full GCC build | hours per cold build (est.; README gives no figure) | must cache built toolchain yourself | Only sensible if you need a bespoke musl/GCC combo; worst CI economics here. |

### Stack-specific notes
- **actions-rs/toolchain is archived** (repo `archived: True`, last push
  2023-06-18 — [GitHub API](https://api.github.com/repos/actions-rs/toolchain)).
  Any migration should replace it with `taiki-e/install-action` (`tool: rust@…`)
  or a `rustup` step / `rust-toolchain.toml`.
- **taiki-e/install-action** covers both `cargo-zigbuild` and `cross` from
  GitHub Releases with checksum + attestation verification
  ([TOOLS.md](https://github.com/taiki-e/install-action/blob/main/TOOLS.md));
  also related:
  [setup-cross-toolchain-action](https://github.com/taiki-e/setup-cross-toolchain-action)
  (QEMU-based cross test env).
- **Swatinem/rust-cache** caches `~/.cargo` + `./target`, keyed by rustc +
  lockfiles + env (`CARGO CC CFLAGS CXX CMAKE RUST` prefixes), 10 GB cap
  ([README](https://github.com/Swatinem/rust-cache)). Note: if you change
  `CC`/`CFLAGS` between jobs (e.g. zig vs musl-gcc), the env-hash key separates
  those caches automatically.

---

## 6. Could not verify (explicit gaps)
1. **Exact wall-clock build times** for this repo's dependency graph under each
   option — no primary source; all times above are estimates.
2. **Whether Zig 0.16's "backported security fixes" include CVE-2025-26519 /
   CVE-2026-6042** — the release notes only explicitly address CVE-2026-40200.
3. **rust-std bundling musl `libc.a` self-contained** — corroborated by
   community sources + rustc book wording ("no special configuration"), but I
   could not inspect the rust-std manifest from this sandbox.
4. **An explicit cross-docs statement that Swatinem/rust-cache works under
   cross** — inferred from CARGO_HOME mount/passthrough docs; not stated.
5. **Current Scaleway ARM pricing** — JS-driven pages didn't yield figures;
   Stardust figure is from 2020 press.
6. **ring 0.17.14 specifically** with zig 0.16 — nearest upstream evidence is
   ring 0.17.13 in zigbuild's CI tests; no issue found for .14 either way.
7. **Hetzner CAX exact current price** — official page is cookie-walled;
   €5.99/mo CAX11 from aggregator + HN threads.

## 7. Bottom line (for the doc's recommendation section)
- The current **zigbuild recipe is sound**; the one mandatory fix is
  **cargo-zigbuild ≥ 0.23.0 with Zig 0.16** (ar/ring bug), and Zig 0.16's
  musl is 1.2.5+backports (better than cross's 1.2.3).
- The biggest structural win is **native `ubuntu-24.04-arm`**: free for public
  repos, native execution removes zig *and* qemu (verification runs the real
  binary), rust-cache unchanged. musl-tools exists on arm64 noble if an
  explicit musl gcc is wanted; otherwise rust-std's bundled musl suffices.
- **cross** buys cross-testing ergonomics at ~437 MB pull + QEMU-sequential
  tests; not compelling for this dependency set.
- **musl.cc is off the table** for GitHub-hosted CI (blocked); building
  musl-cross-make from source is the worst CI economics.

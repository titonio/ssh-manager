# Research: does a static musl binary work on Termux/Android?

**Status: verified.** Every claim below was checked against a primary source on
2026-09-20 — the Termux wiki, the `termux-packages` porting wiki, the musl 1.2.5
source tree (downloaded from musl.libc.org and grepped, not summarised), the
`dirs-sys` and `crossterm` sources, the rustc platform-support book, and live
issue threads in `atuinsh/atuin`, `rust-lang/rustup`, `termux/termux-packages`
and others. Where a claim rests on a forum report rather than a spec, it is
labelled as such.

## Verdict

**Two caveats, not one — and they hit at different moments: the ELF shape blocks *exec*, and outbound DNS blocks *update*.**

As shipped in `v0.1.13`, a Termux user gets **no runnable binary at all**. The
aarch64 asset is linked non-PIE (`Type: EXEC`, fixed base `0x01000000`), and
Android will not exec a non-PIE binary, so the install dies at the pre-swap
`--version` check after a verified download. See the corrected section on
PIE below; this is what `#51` fixes by emitting `static-pie` and asserting
`DYN`.

**Once the PIE fix lands**, a static `aarch64-unknown-linux-musl` binary does
execute on Termux, and sshm's core act survives it, because the core act does
not resolve anything itself — `execute_ssh()` hands the work to Termux's own
bionic OpenSSH ([`sshm/src/ssh.rs`](../sshm/src/ssh.rs)). What still breaks is
every hostname sshm resolves *itself*, which after ADR-0002 means the whole
Apply Update / Check for Updates surface: musl's resolver cannot find a
nameserver on Android, so `sshm update` fails at `getaddrinfo` before it ever
reaches GitHub.

So the `aarch64-unknown-linux-musl` asset added for #48 is **not wrong** —
plain ARM Linux needs it and gets a runnable binary from it — but as shipped
it is **incomplete twice over**: it does not exec on Termux until it is
static-pie, and even then `sshm update` stays dead there. The current README
line "the aarch64 build is the one Termux on Android installs"
([README.md:62-63](../../README.md)) was true about the *download* only, and
is now wrong about even that until `#51` ships.

The rule to carry: **musl-static on Termux is a working TUI with no network of its
own.** Everything that matters for sshm except the updater is unaffected.

## DNS

This is the whole problem, and it is a hard, deterministic failure — not a
flaky one.

**What musl does.** The resolver reads a hardcoded absolute path. From musl
1.2.5, `src/network/resolvconf.c`:

```c
f = __fopen_rb_ca("/etc/resolv.conf", &_f, _buf, sizeof _buf);
if (!f) switch (errno) {
case ENOENT:
case ENOTDIR:
case EACCES:
	goto no_resolv_conf;
```

and the fall-through is the important part:

```c
no_resolv_conf:
	if (!nns) {
		__lookup_ipliteral(conf->ns, "127.0.0.1", AF_UNSPEC);
		nns = 1;
	}
```

**There is no error when the file is missing.** musl silently synthesises a
single nameserver of `127.0.0.1`, with `conf->timeout = 5` and
`conf->attempts = 2` ([same function](https://musl.libc.org/releases/musl-1.2.5.tar.gz)).

**What Android does.** Android has no `/etc/resolv.conf` and no local resolver;
DNS is routed through the `netd` daemon via platform APIs. Termux states this
directly in its own docs:

> Statically linked programs (only networking ones) will not be able to resolve
> DNS names. GNU libc normally doesn't allow static linking with resolver. Also,
> the file /etc/resolv.conf does not exist on Android.
> — [Termux Wiki: Differences from Linux](https://wiki.termux.com/wiki/Differences_from_Linux)

**What actually happens on Termux.** The binary starts, calls `getaddrinfo`,
musl opens `/etc/resolv.conf`, gets `ENOENT`, quietly targets `127.0.0.1:53`,
sends a UDP query to a port nothing is listening on, exhausts its attempts and
returns `EAI_AGAIN` / `EAI_NONAME`
(`src/network/lookup_name.c`: `if (cnt<=0) return cnt ? cnt : EAI_NONAME;`).
Numeric literals still work — the failure is name resolution only, not
connectivity.

The observed error, from a Rust TUI in exactly sshm's position:

```
Error: hub request failed: error sending request for url (https://hub.atuin.sh/auth/cli/code)
Caused by:
   0: error sending request for url (https://hub.atuin.sh/auth/cli/code)
   1: client error (Connect)
   2: dns error
   3: failed to lookup address information: Try again
```
— [atuinsh/atuin#4022](https://github.com/atuinsh/atuin/issues/4022), reporter
on Termux / Android 14 / aarch64, musl build.

The maintainer's diagnosis is the closest available analogue of a ruling:

> It looks like the root cause is that musl builds never have working DNS on
> Termux, because they hardcode `/etc/resolv.conf`. A potential workaround is to
> install Atuin with `pkg install atuin` instead of `install.sh` … In any case,
> our `install.sh` should probably not install binaries with no working DNS on
> Termux.
> — [taylordotfish, atuin#4022](https://github.com/atuinsh/atuin/issues/4022#issuecomment-5485914480)

**Is there a Termux shim?** No, not for this. Termux's `termux-exec` package
patches *shebangs* (`#!/bin/sh` → `$PREFIX/bin/sh`) via a preload shim; it does
nothing for an absolute path baked into a C library. Termux's own answer for
programs that insist on the FHS layout is PRoot:

> If you still need a classical Linux file system layout for some reason, you may
> try to use **termux-chroot** from package 'proot' … these restrictions can be
> bypassed by setting up a Linux distribution rootfs with PRoot.
> — [Termux Wiki: Differences from Linux](https://wiki.termux.com/wiki/Differences_from_Linux)

PRoot is a ptrace-based emulation layer, not Termux — a different runtime with
its own overhead and its own bug surface. It is not "Termux working as advertised".

**What is *not* affected: TLS trust.** A common worry that does not bite here.
Termux's CA bundle lives at
`/data/data/com.termux/files/usr/etc/tls/cert.pem`, not `/etc/ssl/certs`, which
would strand a binary looking for system roots. sshm is immune because its
update dependency pins a compiled-in root store — `webpki-roots` is in the
lockfile next to `reqwest 0.12.28`
([`sshm/Cargo.lock`](../sshm/Cargo.lock)). Certificates are bundled; only the
*name lookup* is missing. (For contrast, rustup's Termux failures include
`SSL error 77 / CA cert inaccessible` and a
`rustls-platform-verifier … Expect rustls-platform-verifier to be initialized`
panic — [rustup#4812](https://github.com/rust-lang/rustup/issues/4812). sshm
uses neither the system CA store nor the platform verifier.)

## User & home lookup

**Android has no `/etc/passwd`, and musl reads exactly that path.**
`src/passwd/getpw_a.c:30` and `getpwent.c:19` both hardcode
`fopen("/etc/passwd", "rbe")`. Under a static musl build, `getpwuid`/`getpwnam`
therefore find nothing. Termux describes the real situation:

> Everything within Termux is executed with the same user id as the Termux
> application itself. The username may look like `u0_a231` and cannot be changed
> as it is derived from the user id by Bionic libc.
> — [Termux Wiki: Differences from Linux](https://wiki.termux.com/wiki/Differences_from_Linux)

Note the asymmetry: that synthesis is a **Bionic** behaviour. A musl-linked binary
does not get it — it goes to the file, and the file is not there.

**Does `dirs` dodge it? Yes, in practice — by ordering, not by magic.**
`dirs-sys::home_dir()` consults the environment first and only then falls back:

```rust
return env::var_os("HOME")
    .and_then(|h| if h.is_empty() { None } else { Some(h) })
    .or_else(|| unsafe { fallback() })   // getpwuid_r
```
— [dirs-sys `src/lib.rs`](https://github.com/dirs-dev/dirs-sys-rs/blob/main/src/lib.rs)

Termux sets `$HOME` to the real home directory:

> The root file system and user home directory are located in a private
> application data directory … Paths to these directories are exposed as
> `$PREFIX` and `$HOME` respectively.
> — [Termux Wiki: Differences from Linux](https://wiki.termux.com/wiki/Differences_from_Linux)

So `dirs::home_dir()` returns `/data/data/com.termux/files/home` on Termux and
sshm's config and `~/.ssh/config` resolution
([`sshm/src/config.rs:68,261`](../sshm/src/config.rs)) work.

**The residual risk is the fallback path, not the normal path.**
`config.rs:68` is `dirs::home_dir().expect("Could not find home directory")`.
If `$HOME` is ever unset or empty — a bare `sh -c`, a harness that clears the
environment, a `termux-exec` misconfiguration — `dirs` falls to `getpwuid_r`,
musl reads the absent `/etc/passwd`, gets `None`, and sshm panics at startup.
Worth noting that `dirs` special-cases this for `target_os = "android"` (its
fallback returns `None` outright), but a musl build is `target_os = "linux"`, so
the `getpwuid_r` path is the one compiled in. sshm already treats an
unresolvable home as `Err` rather than a panic in the newer paths
(`config.rs:272`, `connections.rs:302`); `:68` is the outlier.

No user-visible username is drawn by sshm, so the missing `getpwuid` has no
cosmetic cost either. **This is not a Termux blocker.**

## Exec & linking constraints

**Does a static musl ELF even run on Android?** Yes, and this is the part where
the scary-sounding warnings overstate the case.

The warning in question:

> On non-rooted Android 8 or newer, statically linked programs will not run due
> to issues with seccomp filter.
> — [Termux Wiki: Differences from Linux](https://wiki.termux.com/wiki/Differences_from_Linux)

Read literally, that would make the whole question moot. It is not literally
true. Seccomp is a **per-syscall allowlist applied to app processes**, not a ban
on static binaries; a binary that never issues a blocked syscall simply runs. The
porting wiki describes the actual mechanism:

> Starting from Android 8, a Seccomp was enabled for applications. Seccomp
> forbids usage of some system calls which results in crash with `Bad system
> call` errors.
> — [termux-packages wiki: Common porting problems](https://github.com/termux/termux-packages/wiki/Common-porting-problems)
> (citing [android-developers.googleblog.com: Seccomp filter in Android O](https://android-developers.googleblog.com/2017/07/seccomp-filter-in-android-o.html))

The empirical answer settles it. The atuin reporter runs a **musl-linked Rust TUI
on `Linux 6.1.138-android14 … aarch64 Android`**, non-rooted, and it does not
crash at exec — it runs far enough to open a TLS connection, fail DNS, print a
structured error and produce a `doctor` report
([atuin#4022](https://github.com/atuinsh/atuin/issues/4022)). A binary that
reached `getaddrinfo` cleared the seccomp filter. Same for rustup's musl
`rustup-init` on Termux, which got as far as a `$HOME`-vs-euid check and a
network attempt before failing
([rust-lang forum thread](https://users.rust-lang.org/t/trouble-with-rust-ecosystem-and-toolchain-in-termux/122212)).

**`EXEC_FORMAT` / PIE: CORRECTED — this claim was wrong, and it is the thing that breaks Termux.**

The earlier draft of this section argued that Android's PIE requirement is a
property of `/system/bin/linker64`, the *dynamic* linker, and therefore
"not applicable" to a fully static musl ELF that has no interpreter segment
and never invokes it. The parts of that argument about `termux-elf-cleaner`,
`DT_RPATH`/`DT_RUNPATH` levels and the `LD_LIBRARY_PATH`/`$PREFIX/lib`
conventions hold: a static binary genuinely ignores all of them.

**The conclusion drawn from them does not.** PIE is enforced against static
executables too, and the `v0.1.13` aarch64 asset is the proof:

```
$ file sshm
ELF 64-bit LSB executable, ARM aarch64, version 1 (SYSV), statically linked, stripped
$ readelf -h sshm | grep Type
  Type:   EXEC (Executable file)          # non-PIE, fixed base 0x01000000
```

Installed on Termux that binary downloads, passes its checksum, extracts, and
then fails `--version` — `error: The downloaded binary did not report a
version. Nothing was installed.` Two other projects report the same wall with
the error spelled out, and both are statically linked, which a purely
dynamic-linker property could not explain:

- [bun #28924](https://github.com/oven-sh/bun/issues/28924) — "the install script downloads a non-PIE binary that Android refuses to execute… Android's kernel enforces that all executables must be built as PIE"
- [opencode #10504](https://github.com/anomalyco/opencode/issues/10504) — `error: Android 5.0 and later only support position-independent executables (-fPIE)` under `/data/data/com.termux/`

The x86_64 musl leg never showed this because it gets PIE by accident:
Ubuntu's GCC defaults to PIE, so `cargo build` against musl-tools emits
`DYN`/`static-pie`. Zig links aarch64 musl as non-PIE by default.

**Why the existing CI gate could not catch it.** `qemu-aarch64-static ./sshm --version`
returns `sshm 0.1.13` and exit 0 against that exact non-PIE binary. qemu
emulates the CPU and the Linux syscalls and has no opinion about Android's
PIE policy, so the gate proved *"this is a working aarch64 binary"* — true —
and never asked *"is this executable on Android"* — false. The ELF header
type is the cheap place to ask, and `#51` asserts `DYN` there.

**What this does not settle.** The DNS finding below stands independently and
is still the harder problem: fixing PIE makes the binary *run*, it does not
make `sshm update` resolve a hostname on Android.

**`LD_PRELOAD`: irrelevant.** A static binary has no dynamic symbol resolution to
preempt. Termux's `termux-exec` preload shim cannot inject anything into it —
which is also why no Termux compat layer rescues the DNS path.

**One TUI-specific landmine that sshm happens to have dodged.** Termux documents:

> Starting from Android 8, programs cannot use `tcsetattr()` with `TCSAFLUSH`
> parameter due to SELinux. Use `TCSANOW` instead.
> — [Common porting problems](https://github.com/termux/termux-packages/wiki/Common-porting-problems)

That is a raw-mode-killer for a Ratatui app. Checked against the vendored
dependency: crossterm 0.28.1 uses `TCSANOW` on both code paths —
`src/terminal/sys/unix.rs:305` (`tcsetattr(fd, TCSANOW, termios)`) and
`:177` (`rustix::termios::OptionalActions::Now`). **sshm's raw mode is safe on
Termux.** This is luck, not design; it is worth knowing before a crossterm
upgrade changes it.

**Android 9+ setuid**: blocked by seccomp
([Common porting problems](https://github.com/termux/termux-packages/wiki/Common-porting-problems)).
sshm never calls setuid and its installer deliberately avoids escalation where
the directory is already writable (`install.sh:choose_privilege`), so nothing to
do.

## What other projects do

The pattern across ecosystems is consistent: **the static binary runs, the DNS
doesn't, and the projects that solved it stopped shipping the static binary to
Termux.**

| Project | What they ship | What breaks on Termux | What they did |
|---|---|---|---|
| **atuin** (Rust TUI — closest analogue) | musl static via `install.sh` | All HTTPS: `dns error … Try again` | Issue open; maintainer: `install.sh` "should probably not install binaries with no working DNS on Termux"; workaround is `pkg install atuin` ([#4022](https://github.com/atuinsh/atuin/issues/4022)). A native bionic `atuin` **does** exist in termux-main ([build.sh](https://github.com/termux/termux-packages/blob/master/packages/atuin/build.sh)) |
| **rustup** | musl static installer | DNS, then CA store, then platform-verifier panic | Termux marked the request `wontfix`: "`rustup` isn't supported on Termux. Rust requires a significant amount of patches…" — use `pkg i rust` or TUR `rustc-nightly` ([termux-packages#24226](https://github.com/termux/termux-packages/issues/24226)); rustup member: "Termux host is not supported by the rust project" ([rustup#4812](https://github.com/rust-lang/rustup/issues/4812)) |
| **Crystal** | Native Termux package, not a static blob | — (avoided it) | Ships via termux-packages against bionic; maintainer: "static linking on Android is not recommended or really useful. Statically linked executables are not exactly portable, they can break between Android versions" ([Crystal forum](https://forum.crystal-lang.org/t/crystal-is-now-available-on-termux-aarch64/5837)) |
| **Go programs** | `CGO_ENABLED=0` static | `lookup api.openai.com on [::1]:53: connection refused` — the same localhost-fallback signature | Rebuild with `CGO_ENABLED=1` + Android NDK so the bionic resolver is used: "Go's pure DNS resolver cannot work on Android" ([dave.engineer](https://dave.engineer/blog/2025/11/cross-compiling-go-android/)) |
| **kestrel-agent** (Rust + reqwest) | musl static | All HTTPS, 5s timeouts | Fixed in-process by enabling reqwest's `hickory-dns` resolver: "bypasses musl getaddrinfo entirely … No behavior regression on any platform" ([#163](https://github.com/Bahtya/kestrel-agent/issues/163)) |
| **r/termux users** | various static binaries | "issues with DNS resolution when using statically linked binary on Android 8" | Community guidance is: don't static-link on Android ([thread](https://www.reddit.com/r/termux/comments/lpltwc/statically_linking_binaries/)) |

**On Rust's own support.** `*-linux-android` is a **Tier 2** target: std is
supported, ELF output, and it "is cross-compiled from a host environment … using
the Android NDK"
([rustc book: platform support](https://doc.rust-lang.org/nightly/rustc/platform-support/android.html)).
So a real Android build is supported by Rust — but it requires the NDK in CI, and
Termux's own Rust is a heavily patched source build that ships only the four
android targets
([termux-packages/packages/rust/build.sh](https://github.com/termux/termux-packages/blob/master/packages/rust/build.sh)).
`aarch64-unknown-linux-musl` and `aarch64-linux-android` are genuinely different
builds; there is no flag that makes the former behave like the latter.

**The important distinction for sshm.** Every project above broke because its
*own* network calls were the product. sshm's product is "pick a connection,
exec `ssh`" — and the thing that resolves the SSH hostname is Termux's bionic
OpenSSH, not sshm. That is why sshm's core is fine on the musl asset while
atuin's core is not.

## Options for sshm

| Option | What it costs | Who it serves | Recommendation |
|---|---|---|---|
| **(a) Termux-native bionic build** (`aarch64-linux-android`) | Highest. New NDK cross-toolchain in CI alongside the existing zig/cargo-zigbuild musl legs; a third asset name; `install.sh` and `get_platform_asset_name()` both gain a Termux branch; unknown interaction with `ring`/`rustls` under NDK; Termux's own Rust needs a patch series, which is a signal about how much friction lives here. Also does **not** make sshm installable via `pkg` — it is still a downloaded blob. | Termux users who want working `sshm update`. | **No.** Buys the least for the most CI surface, and (e) gets most of the same benefit for a fraction of it. |
| **(b) Package in termux-main or TUR** | Medium, and partly outside this repo's control. A `build.sh` against bionic; termux-main acceptance is selective, TUR is open (`pkg install tur-repo`, [TUR README](https://github.com/termux-user-repository/tur/blob/master/README.md)). Proven feasible for exactly this shape — atuin is in termux-main. Ongoing: every release needs a package bump, and TUR explicitly asks that issues not be filed in Termux's official forum. | Termux users who want a first-class, updatable-via-`pkg` install. | **Yes, as the durable follow-up — not for v0.1.13.** This is the answer atuin's maintainer points at, and the only one that makes Termux a supported platform rather than a tolerated one. |
| **(c) Ship musl, document the Termux caveats** | Lowest. A paragraph in README + this doc. But note it is only honest if the caveat is *specific*: "Termux is supported" is false; "the picker and `ssh` work, `sshm update` does not" is true. | Everyone; costs nothing; risks under-informing. | **Yes, minimum bar.** Do it regardless of what else is chosen. |
| **(d) Detect Termux in `install.sh` and route/warn** | Low — ~10 lines in `detect_platform()`. Termux is unambiguous to detect: `$PREFIX` contains `com.termux` (or `$TERMUX_APP_PACKAGE` / `$TERMUX_UID` are set), which plain ARM Linux never has. Cheapest form is a warning, not a different asset: keep installing the musl binary, print that Apply Update will not work and why. A *routing* variant needs (a) or (b) to exist first. | Termux users, at the exact moment the choice is made — instead of three commands later, mid-error. | **Yes — recommended now.** It converts a silent dead end into a stated boundary, and it is the change that fits what `install.sh` already does. |
| **(e) Fix DNS in-process: reqwest `hickory-dns`** | Low-to-medium, and the only option that actually *fixes* the failure rather than describing it. reqwest 0.12 has a `hickory-dns` feature; sshm's reqwest is transitive via `self-github-update-enhanced`, so the fix is either asking upstream to expose it, or adding a direct `reqwest = { version = "0.12", features = ["hickory-dns"] }` and letting Cargo feature unification apply it to the shared copy. Unverified here: whether hickory's own config discovery works on Termux (it also reads `resolv.conf` by default — it must be configured with an explicit upstream resolver, e.g. `Config::from_str` with a public nameserver, not `tokio_from_system_conf`). | Termux users who want `sshm update` to work with no extra install step. | **Worth one spike.** Precedent exists ([kestrel-agent#163](https://github.com/Bahtya/kestrel-agent/issues/163)). Do not block v0.1.13 on it — the "no regression on any platform" claim needs a real Termux test, and a hard-coded public resolver is a policy decision, not just a feature flag. |

**Recommended for this repo, in order:**

1. **Ship v0.1.13 as planned.** The aarch64 musl asset is a strict improvement
   over #48's state and is correct for plain ARM Linux. Do not revert it.
2. **Do (c) + (d) now.** Detect Termux in `install.sh` and state the boundary in
   the README: the picker and `ssh` exec work; `sshm update` / `check-update`
   cannot resolve GitHub under musl on Termux; use `install.sh` to update there.
   This is a documentation-and-installer change, no Rust code, no new asset.
3. **Open an issue for (e)** as the real fix, scoped as a spike with a Termux
   test attached.
4. **Track (b) as the durable answer** when someone has appetite for a package
   bump per release.

## Open questions

1. **Does `hickory-resolver` actually resolve on Termux?** Its default config
   discovery reads `resolv.conf` too. The kestrel-agent report claims the feature
   flag alone fixes it, but that report is a proposal, not a verified Termux
   test. Needs: build with `hickory-dns`, run `sshm check-update` on real
   Termux, confirm.
2. **Which upstream owns the fix.** `self-github-update-enhanced` controls the
   reqwest feature set. Whether it will expose `hickory-dns` (or whether feature
   unification via a direct dev-neutral dependency works) is untested here.
3. **Exact seccomp allowlist per API level.** The Termux wiki's "statically
   linked programs will not run" claim was not verified against the AOSP
   `libc/seccomp/app_policy_*.txt` in this pass — the fetch failed. The
   empirical evidence (atuin running on Android 14) contradicts the blanket
   reading, but a specific syscall sshm might add later (a new crate using
   `clone3`, `pidfd_open`, `landlock_*`, `userfaultfd`) could still trip it.
   Worth pinning the policy list before sshm grows a new syscall-using dependency.
4. **Does the `ring`/`aarch64` asm path behave under Android's kernel?** CI
   verifies the aarch64 asset only under `qemu-aarch64-static`, which — as
   #48's own triage note conceded — "exercises the ELF/exec path but not
   Android's kernel seccomp filter". Still unverified on device.
5. **IPv6-only and IPv6-broken networks.** The atuin reporter noted the client
   "does not fall back to IPv4 when IPv6 is unreachable, unlike curl". Whether
   that is musl-specific or reqwest/hyper behaviour was not determined, and it
   would survive a DNS fix.
6. **`$HOME` unset edge case.** `config.rs:68`'s `.expect()` is the one place
   where musl's missing `/etc/passwd` could turn a missing env var into a panic.
   Not Termux-specific, but Termux is where it is most likely to be hit.

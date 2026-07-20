# Vendored: garmin_client 0.2.1

| | |
|---|---|
| Upstream | https://github.com/poster515/Rust-Garmin/ |
| crates.io | https://crates.io/crates/garmin_client |
| Version | 0.2.1 |
| **License** | **GPL-3.0** — see the note below |

## Why this is vendored

Upstream reads the MFA code from stdin:

```rust
print!("Enter MFA code: ");
stdin().read_line(&mut mfa_code)
```

That works from a shell, but it cannot work under MCP. This server communicates
over stdio, so stdin carries the JSON-RPC transport — the prompt consumes
protocol bytes and login fails with an empty code. Any Garmin account with
two-factor authentication enabled is therefore unusable with the stock crate,
and there is no way to supply the code from outside: `login()` is the only
public entry point and `handle_mfa()` is private.

## What changed

Two files differ from the published crate.

**`src/totp.rs`** — new. TOTP generation per RFC 6238 (HMAC-SHA1, 30-second
period, six digits), checked against the test vectors published in RFC 6238
Appendix B.

**`src/lib.rs`** — `handle_mfa` now resolves the code from three sources in
order instead of always prompting:

1. `GARMIN_MFA_CODE` — an explicit code, for a single attended run.
2. `GARMIN_TOTP_SECRET` — the account's base32 two-factor secret. Fully
   unattended; this is the option that makes MCP work with 2FA enabled.
3. stdin — kept for interactive use, but only when stdin is actually a
   terminal (`IsTerminal`). Where it is not, the error explains what to set
   rather than silently failing on an empty read.

Also fixed while here: the stdin path passed the code through with its trailing
newline attached. It is now trimmed.

Everything else is byte-for-byte the published 0.2.1 source.

## Licensing — read this before distributing

`garmin_client` is **GPL-3.0**. This repository is otherwise Apache-2.0.

Apache-2.0 and GPL-3.0 are compatible in one direction only: Apache-2.0 code may
be incorporated into a GPL-3.0 work, not the reverse. A combined work that
includes this source is therefore GPL-3.0 when distributed, and the repository's
Apache-2.0 declaration does not override that.

Note that this obligation predates the vendoring — the crate was already a
dependency, and linking a GPL-3.0 library carries the same requirement. Copying
the source in makes the situation visible rather than creating it.

## Preferred long-term fix

Send the MFA change upstream. If it is accepted and released, this directory can
be deleted and `Cargo.toml` can go back to a plain crates.io dependency, which
removes the maintenance burden of tracking a fork.

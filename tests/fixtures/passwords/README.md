# Public password-hash fixtures

`php-argon2id.json` was generated and verified locally with PHP 8.4.20 using
`password_hash($password, PASSWORD_ARGON2ID, $options)` and `password_verify`.
The generator version, public passwords, options, and `password_get_info` results
are stored beside each hash. The final case uses PHP's default options.

These are synthetic public test inputs, not credentials. Salts came from PHP's
OS-backed generator; regenerating fixtures changes the hashes. To regenerate,
retain the listed cases/passwords/options, call the functions above, and encode
the resulting object as UTF-8 JSON. No PHP executable is needed by Cargo tests.

The additional reference vector in `crates/scr-stdlib/tests/password.rs` comes
from the Argon2 reference implementation, as reproduced in RustCrypto argon2
0.5.3 `tests/phc_strings.rs`:
https://docs.rs/crate/argon2/0.5.3/source/tests/phc_strings.rs

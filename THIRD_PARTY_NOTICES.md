# Third-party notices

Mocky Mail is licensed under the GNU General Public License, version 3 or
later. It includes the following code under other licences, which allow it.

## MailCraft

Crate used: `mailcraft-openpgp` (checking OpenPGP signatures, reading
OpenPGP keys and Autocrypt headers), from
<https://github.com/UCSLabs/mailcraft>, pinned to commit
`61e9f46f069a3f0cf016283d7c329c07bd60c7cc`. It is dual-licensed MIT OR
Apache-2.0; Mocky Mail uses it under the MIT licence:

```text
MIT License

Copyright (c) 2026 the MailCraft contributors

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

## rPGP

Crate used: `pgp` 0.21.1 (OpenPGP signing, encryption and decryption), from
<https://github.com/rpgp/rpgp>. Dual-licensed MIT OR Apache-2.0; Mocky Mail
uses it under the MIT licence:

```text
Copyright (c) the rPGP developers

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

## RustCrypto

Crates used for S/MIME and the algorithms both kinds need: `cms`, `x509-cert`,
`der`, `spki`, `const-oid`, `pkcs1`, `pkcs5`, `pkcs8`, `pkcs12`, `rsa`,
`p256`, `p384`, `signature`, `sha1`, `sha2`, `hmac`, `aes`, `aes-gcm`, `cbc`,
`des`, `rc2`, `rand_core`, `zeroize`, from <https://github.com/RustCrypto>.
Dual-licensed Apache-2.0 OR MIT; Mocky Mail uses them under the MIT licence:

```text
Copyright (c) 2021 The RustCrypto Project Developers

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

## Other crates

The other Rust crates Mocky Mail is built from are listed, with their
licences, in `Cargo.lock` and on crates.io. Among those added with this
version: `rustls-native-certs` (Apache-2.0 OR ISC OR MIT; the system's trusted
roots on macOS and Linux), `schannel` (MIT; Windows' certificate stores), and,
for tests only, `rcgen` (MIT OR Apache-2.0).

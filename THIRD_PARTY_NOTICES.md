# Third-party notices

Mocky Mail is licensed under the GNU General Public License, version 3 or
later. It includes the following code under other licences, which allow it.

## MailCraft

Crates used: `mailcraft-pst` (Outlook `.pst` files), `mailcraft-tnef`
(`winmail.dat` attachments) and `mailcraft-charset` (character sets of mail
and contact files), from <https://github.com/UCSLabs/mailcraft>, pinned to
commit `61e9f46f069a3f0cf016283d7c329c07bd60c7cc`. They are dual-licensed
MIT OR Apache-2.0; Mocky Mail uses them under the MIT licence:

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

The way the `.pst` import turns Outlook items into messages, contacts and
events follows MailCraft's own import (`mailcraft-engine`), under the same
licence.

## Other crates

The other Rust crates Mocky Mail is built from are listed, with their
licences, in `Cargo.lock` and on crates.io. Among those added with this
version: `zip` (MIT), for exporting messages as `.eml` files in a `.zip`,
and `encoding_rs` ((Apache-2.0 OR MIT) AND BSD-3-Clause), through
`mail-parser`'s `full_encoding` and `mailcraft-charset`, for mail in
Shift_JIS, GBK, Big5, EUC-KR and the other multi-byte character sets.

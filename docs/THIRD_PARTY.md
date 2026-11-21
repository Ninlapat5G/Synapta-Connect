# ส่วนประกอบภายนอกและเงื่อนไข (Synapta Connect v0.3.0)

ตรวจจาก `cargo metadata` ของเป้าหมาย Windows x86_64 (เฉพาะ dependency ที่ถูก build เข้าไบนารีจริง ไม่รวม dev-dependency) เมื่อ 2026-11-20
**ไม่ใช่คำแนะนำทางกฎหมาย** — เป็นผลตรวจทางเทคนิคเพื่อให้เจ้าของตัดสินใจ

## สรุป

- ส่วนประกอบ Rust ที่ถูกลิงก์เข้า exe: **271 crate** ทุกตัวมีสัญญาอนุญาตระบุชัดใน metadata (ไม่มีตัวที่ไม่ระบุ)
- **ไม่พบ copyleft** (GPL / LGPL / AGPL / MPL / EPL / CDDL / SSPL / CC-BY-SA) — ทุกตัวเป็นตระกูลอนุญาตเสรี: MIT, Apache-2.0, BSD-2/3-Clause, ISC, Zlib, 0BSD, Unlicense, BSL-1.0, Unicode-3.0 (หลายตัวเป็น "A OR B" เลือกข้อใดข้อหนึ่งได้)
- การเผยแพร่ **ไบนารี** (exe) ที่ฝังโค้ดเหล่านี้ ต้อง **แนบประกาศลิขสิทธิ์และข้อความสัญญาอนุญาตของแต่ละตัว** (เงื่อนไขร่วมของ MIT/BSD/ISC/Apache-2.0/Unicode-3.0) — Apache-2.0 ต้องแนบ NOTICE ของโปรเจกต์นั้นด้วยถ้ามี
- การที่ซอร์สของเราเป็น "สงวนสิทธิ์" **ไม่ขัด** กับสัญญาอนุญาตเหล่านี้ (ทั้งหมดอนุญาตให้ใช้ในซอฟต์แวร์ปิด/สงวนสิทธิ์) แต่สัญญาของเรา (`LICENSE`) ไม่ครอบคลุมโค้ดของเขา — แจ้งไว้ใน LICENSE แล้ว

## ประกาศและเงื่อนไขก่อนแจกจ่ายไบนารี

1. แนบ [THIRD_PARTY_NOTICES.txt](THIRD_PARTY_NOTICES.txt) กับไบนารี ข้างในมีข้อความสัญญาอนุญาตและประกาศจาก 271 crate ในตาราง พร้อมประกาศของ Rust standard library สำหรับ Windows x86_64
2. **ตัวที่ควรอ่านสัญญาจริงเป็นพิเศษ**: `ring` (Apache-2.0 AND ISC — โค้ดส่วนหนึ่งมาจาก BoringSSL/OpenSSL มีประกาศเพิ่มในไฟล์ LICENSE ของ ring), `rustls-webpki` / `untrusted` (ISC), `curve25519-dalek` / `subtle` (BSD-3-Clause: ห้ามใช้ชื่อผู้พัฒนาโฆษณา), `icu_*` (Unicode-3.0: ต้องแนบประกาศข้อมูล Unicode), `xcap` (Apache-2.0), `matchit` (MIT AND BSD-3-Clause)
3. **รูปใน `frontend/assets/`** (`hero.jpg`, `control.jpg`, `synapta-tech.webp`): ตรวจที่มาจากไฟล์ไม่ได้ — **เจ้าของต้องยืนยัน** ว่าเป็นงานของตนหรือได้สิทธิ์แจกจ่ายใน public repo (ถ้าสร้างด้วย AI หรือสต็อก ต้องดูเงื่อนไขของแหล่งนั้น) — รายละเอียดใน `docs/RELEASE.md` §2 ข้อ 6
4. ชื่อเจ้าของลิขสิทธิ์ใน `LICENSE` ยังเป็นที่ว่าง

## ส่วนประกอบที่ไม่ใช่ crate

| รายการ | สถานะ |
|---|---|
| หน้าเว็บ panel (`frontend/`) | โค้ดเขียนเองทั้งหมด ไม่มี JS/CSS ของบุคคลที่สามและไม่โหลดอะไรจากอินเทอร์เน็ตตอนรัน (ตรวจ: ไม่มี `import`/URL ภายนอกใน `app.js`) |
| ฟอนต์ | [Anuphan](https://fonts.google.com/specimen/Anuphan) และ [Chakra Petch](https://fonts.google.com/specimen/Chakra+Petch) จาก Google Fonts ใช้ SIL Open Font License 1.1 ไฟล์ WOFF2 และไลเซนส์อยู่ใน `frontend/assets/fonts/` และโหลดจาก agent บนเครื่อง |
| Microsoft Visual C++ runtime | build ใช้ CRT แบบ static (`backend/.cargo/config.toml`) — exe ไม่ต้องติดตั้ง VC++ Redistributable และเราไม่ได้แจกจ่ายไฟล์ runtime ของ Microsoft |
| Windows API (SCM, icacls, netsh, reg, sc, powershell) | เรียกเครื่องมือที่มากับ Windows ตอนติดตั้ง/ถอน ไม่แนบ/ไม่แจกจ่าย |
| Rust standard library | MIT OR Apache-2.0 (ลิงก์เข้า exe) — รวมในประกาศฉบับเต็มด้วย |
| บริการภายนอกตอนรัน | GitHub (api.github.com / github.com / objects.githubusercontent.com) เฉพาะเมื่อเปิดอัปเดตและตั้ง source เป็น `github:`; ข้อกำหนดของ GitHub ใช้กับการเรียก API |
| เครื่องมือ build | `rustc`/`cargo` (ไม่ถูกแจกจ่าย) |

## รายการ crate ที่ถูกลิงก์ (สร้างจาก metadata)

| crate | รุ่น | สัญญาอนุญาต | ที่มา |
|---|---|---|---|
| adler2 | 2.0.1 | 0BSD OR MIT OR Apache-2.0 | https://github.com/oyvindln/adler2 |
| aho-corasick | 1.1.5 | Unlicense OR MIT | https://github.com/BurntSushi/aho-corasick |
| anyhow | 1.0.104 | MIT OR Apache-2.0 | https://github.com/dtolnay/anyhow |
| asn1-rs | 0.7.2 | MIT OR Apache-2.0 | https://github.com/rusticata/asn1-rs.git |
| asn1-rs-derive | 0.6.0 | MIT OR Apache-2.0 | https://github.com/rusticata/asn1-rs.git |
| asn1-rs-impl | 0.2.0 | MIT/Apache-2.0 | https://github.com/rusticata/asn1-rs.git |
| atomic-waker | 1.1.2 | Apache-2.0 OR MIT | https://github.com/smol-rs/atomic-waker |
| autocfg | 1.5.1 | Apache-2.0 OR MIT | https://github.com/cuviper/autocfg |
| axum | 0.8.9 | MIT | https://github.com/tokio-rs/axum |
| axum-core | 0.5.6 | MIT | https://github.com/tokio-rs/axum |
| base64 | 0.22.1 | MIT OR Apache-2.0 | https://github.com/marshallpierce/rust-base64 |
| base64 | 0.23.1 | MIT OR Apache-2.0 | https://github.com/marshallpierce/rust-base64 |
| bit-vec | 0.9.1 | Apache-2.0 OR MIT | https://github.com/contain-rs/bit-vec |
| bitflags | 1.3.2 | MIT/Apache-2.0 | https://github.com/bitflags/bitflags |
| bitflags | 2.13.2 | MIT OR Apache-2.0 | https://github.com/bitflags/bitflags |
| block-buffer | 0.10.4 | MIT OR Apache-2.0 | https://github.com/RustCrypto/utils |
| block-buffer | 0.12.1 | MIT OR Apache-2.0 | https://github.com/RustCrypto/utils |
| bstr | 1.14.0 | MIT OR Apache-2.0 | https://github.com/BurntSushi/bstr |
| bytemuck | 1.25.2 | Zlib OR Apache-2.0 OR MIT | https://github.com/Lokathor/bytemuck |
| bytemuck_derive | 1.12.1 | Zlib OR Apache-2.0 OR MIT | https://github.com/Lokathor/bytemuck |
| byteorder-lite | 0.1.0 | Unlicense OR MIT | https://github.com/image-rs/byteorder-lite |
| bytes | 1.12.2 | MIT | https://github.com/tokio-rs/bytes |
| cc | 1.11.2 | MIT OR Apache-2.0 | https://github.com/rust-lang/cc-rs |
| cfg-if | 1.0.5 | MIT OR Apache-2.0 | https://github.com/rust-lang/cfg-if |
| cfg_aliases | 0.1.1 | MIT | https://github.com/katharostech/cfg_aliases |
| chacha20 | 0.10.2 | MIT OR Apache-2.0 | https://github.com/RustCrypto/stream-ciphers |
| cmov | 0.5.4 | Apache-2.0 OR MIT | https://github.com/RustCrypto/utils |
| const-oid | 0.10.3 | Apache-2.0 OR MIT | https://github.com/RustCrypto/formats |
| cpufeatures | 0.2.17 | MIT OR Apache-2.0 | https://github.com/RustCrypto/utils |
| cpufeatures | 0.3.1 | MIT OR Apache-2.0 | https://github.com/RustCrypto/utils |
| crc32fast | 1.5.5 | MIT OR Apache-2.0 | https://github.com/srijs/rust-crc32fast |
| crypto-common | 0.1.7 | MIT OR Apache-2.0 | https://github.com/RustCrypto/traits |
| crypto-common | 0.2.2 | MIT OR Apache-2.0 | https://github.com/RustCrypto/traits |
| ctutils | 0.4.8 | Apache-2.0 OR MIT | https://github.com/RustCrypto/utils |
| curve25519-dalek | 4.1.3 | BSD-3-Clause | https://github.com/dalek-cryptography/curve25519-dalek/tree/main/curve25519-dalek |
| curve25519-dalek-derive | 0.1.1 | MIT/Apache-2.0 | https://github.com/dalek-cryptography/curve25519-dalek |
| data-encoding | 2.11.1 | MIT | https://github.com/ia0/data-encoding |
| der-parser | 10.0.0 | MIT OR Apache-2.0 | https://github.com/rusticata/der-parser.git |
| deranged | 0.5.9 | MIT OR Apache-2.0 | https://github.com/jhpratt/deranged |
| digest | 0.10.7 | MIT OR Apache-2.0 | https://github.com/RustCrypto/traits |
| digest | 0.11.3 | MIT OR Apache-2.0 | https://github.com/RustCrypto/traits |
| displaydoc | 0.2.7 | MIT OR Apache-2.0 | https://github.com/yaahc/displaydoc |
| downcast-rs | 1.2.1 | MIT/Apache-2.0 | https://github.com/marcianx/downcast-rs |
| enigo | 0.6.1 | MIT | https://github.com/enigo-rs/enigo |
| equivalent | 1.0.3 | Apache-2.0 OR MIT | https://github.com/indexmap-rs/equivalent |
| fastrand | 2.5.0 | Apache-2.0 OR MIT | https://github.com/smol-rs/fastrand |
| fdeflate | 0.3.7 | MIT OR Apache-2.0 | https://github.com/image-rs/fdeflate |
| filedescriptor | 0.8.3 | MIT | https://github.com/wezterm/wezterm |
| find-msvc-tools | 0.1.15 | MIT OR Apache-2.0 | https://github.com/rust-lang/cc-rs |
| flate2 | 1.1.11 | MIT OR Apache-2.0 | https://github.com/rust-lang/flate2-rs |
| flume | 0.12.0 | Apache-2.0/MIT | https://github.com/zesterer/flume |
| fnv | 1.0.7 | Apache-2.0 / MIT | https://github.com/servo/rust-fnv |
| form_urlencoded | 1.2.2 | MIT OR Apache-2.0 | https://github.com/servo/rust-url |
| futures-channel | 0.3.34 | MIT OR Apache-2.0 | https://github.com/rust-lang/futures-rs |
| futures-core | 0.3.34 | MIT OR Apache-2.0 | https://github.com/rust-lang/futures-rs |
| futures-io | 0.3.34 | MIT OR Apache-2.0 | https://github.com/rust-lang/futures-rs |
| futures-macro | 0.3.34 | MIT OR Apache-2.0 | https://github.com/rust-lang/futures-rs |
| futures-sink | 0.3.34 | MIT OR Apache-2.0 | https://github.com/rust-lang/futures-rs |
| futures-task | 0.3.34 | MIT OR Apache-2.0 | https://github.com/rust-lang/futures-rs |
| futures-util | 0.3.34 | MIT OR Apache-2.0 | https://github.com/rust-lang/futures-rs |
| generic-array | 0.14.7 | MIT | https://github.com/fizyk20/generic-array.git |
| getrandom | 0.2.17 | MIT OR Apache-2.0 | https://github.com/rust-random/getrandom |
| getrandom | 0.3.4 | MIT OR Apache-2.0 | https://github.com/rust-random/getrandom |
| getrandom | 0.4.3 | MIT OR Apache-2.0 | https://github.com/rust-random/getrandom |
| globset | 0.4.21 | Unlicense OR MIT | https://github.com/BurntSushi/ripgrep/tree/master/crates/globset |
| h2 | 0.4.20 | MIT | https://github.com/hyperium/h2 |
| hashbrown | 0.17.1 | MIT OR Apache-2.0 | https://github.com/rust-lang/hashbrown |
| hkdf | 0.12.4 | MIT OR Apache-2.0 | https://github.com/RustCrypto/KDFs/ |
| hmac | 0.12.1 | MIT OR Apache-2.0 | https://github.com/RustCrypto/MACs |
| hmac | 0.13.0 | MIT OR Apache-2.0 | https://github.com/RustCrypto/MACs |
| http | 1.5.0 | MIT OR Apache-2.0 | https://github.com/hyperium/http |
| http-body | 1.1.0 | MIT | https://github.com/hyperium/http-body |
| http-body-util | 0.1.5 | MIT | https://github.com/hyperium/http-body |
| http-range-header | 0.4.2 | MIT | https://github.com/MarcusGrass/parse-range-headers |
| httparse | 1.10.1 | MIT OR Apache-2.0 | https://github.com/seanmonstar/httparse |
| httpdate | 1.0.3 | MIT OR Apache-2.0 | https://github.com/pyfisch/httpdate |
| hybrid-array | 0.4.15 | MIT OR Apache-2.0 | https://github.com/RustCrypto/hybrid-array |
| hyper | 1.12.0 | MIT | https://github.com/hyperium/hyper |
| hyper-rustls | 0.27.10 | Apache-2.0 OR ISC OR MIT | https://github.com/rustls/hyper-rustls |
| hyper-util | 0.1.21 | MIT | https://github.com/hyperium/hyper-util |
| icu_collections | 2.4.0 | Unicode-3.0 | https://github.com/unicode-org/icu4x |
| icu_locale_core | 2.4.0 | Unicode-3.0 | https://github.com/unicode-org/icu4x |
| icu_normalizer | 2.4.0 | Unicode-3.0 | https://github.com/unicode-org/icu4x |
| icu_normalizer_data | 2.4.0 | Unicode-3.0 | https://github.com/unicode-org/icu4x |
| icu_properties | 2.4.0 | Unicode-3.0 | https://github.com/unicode-org/icu4x |
| icu_properties_data | 2.4.0 | Unicode-3.0 | https://github.com/unicode-org/icu4x |
| icu_provider | 2.4.0 | Unicode-3.0 | https://github.com/unicode-org/icu4x |
| idna | 1.1.0 | MIT OR Apache-2.0 | https://github.com/servo/rust-url/ |
| idna_adapter | 1.2.2 | Apache-2.0 OR MIT | https://github.com/hsivonen/idna_adapter |
| if-addrs | 0.15.0 | MIT OR BSD-3-Clause | https://github.com/messense/if-addrs |
| image | 0.25.10 | MIT OR Apache-2.0 | https://github.com/image-rs/image |
| image-webp | 0.2.4 | MIT OR Apache-2.0 | https://github.com/image-rs/image-webp |
| indexmap | 2.14.2 | Apache-2.0 OR MIT | https://github.com/indexmap-rs/indexmap |
| ipnet | 2.12.2 | MIT OR Apache-2.0 | https://github.com/krisprice/ipnet |
| is_elevated | 0.1.2 | MIT | https://github.com/yandexx/is_elevated |
| itoa | 1.0.19 | MIT OR Apache-2.0 | https://github.com/dtolnay/itoa |
| lazy_static | 1.5.1 | MIT OR Apache-2.0 | https://github.com/rust-lang-nursery/lazy-static.rs |
| libc | 0.2.191 | MIT OR Apache-2.0 | https://github.com/rust-lang/libc |
| litemap | 0.8.3 | Unicode-3.0 | https://github.com/unicode-org/icu4x |
| lock_api | 0.4.14 | MIT OR Apache-2.0 | https://github.com/Amanieu/parking_lot |
| log | 0.4.35 | MIT OR Apache-2.0 | https://github.com/rust-lang/log |
| matchers | 0.2.0 | MIT | https://github.com/hawkw/matchers |
| matchit | 0.8.4 | MIT AND BSD-3-Clause | https://github.com/ibraheemdev/matchit |
| mdns-sd | 0.22.0 | Apache-2.0 OR MIT | https://github.com/keepsimple1/mdns-sd |
| memchr | 2.8.3 | Unlicense OR MIT | https://github.com/BurntSushi/memchr |
| mime | 0.3.18 | MIT OR Apache-2.0 | https://github.com/hyperium/mime |
| mime_guess | 2.0.5 | MIT | https://github.com/abonander/mime_guess |
| minimal-lexical | 0.2.1 | MIT/Apache-2.0 | https://github.com/Alexhuszagh/minimal-lexical |
| miniz_oxide | 0.8.9 | MIT OR Zlib OR Apache-2.0 | https://github.com/Frommi/miniz_oxide/tree/master/miniz_oxide |
| miniz_oxide | 0.9.1 | MIT OR Zlib OR Apache-2.0 | https://github.com/Frommi/miniz_oxide/tree/master/miniz_oxide |
| mio | 1.2.5 | MIT | https://github.com/tokio-rs/mio |
| moxcms | 0.8.1 | BSD-3-Clause OR Apache-2.0 | https://github.com/awxkee/moxcms.git |
| nix | 0.28.0 | MIT | https://github.com/nix-rust/nix |
| nom | 7.1.3 | MIT | https://github.com/Geal/nom |
| ntapi | 0.4.3 | Apache-2.0 OR MIT | https://github.com/MSxDOS/ntapi |
| nu-ansi-term | 0.50.3 | MIT | https://github.com/nushell/nu-ansi-term |
| num-bigint | 0.4.8 | MIT OR Apache-2.0 | https://github.com/rust-num/num-bigint |
| num-conv | 0.2.2 | MIT OR Apache-2.0 | https://github.com/jhpratt/num-conv |
| num-integer | 0.1.47 | MIT OR Apache-2.0 | https://github.com/rust-num/num-integer |
| num-traits | 0.2.19 | MIT OR Apache-2.0 | https://github.com/rust-num/num-traits |
| oid-registry | 0.8.1 | MIT OR Apache-2.0 | https://github.com/rusticata/oid-registry.git |
| once_cell | 1.21.4 | MIT OR Apache-2.0 | https://github.com/matklad/once_cell |
| parking_lot | 0.12.5 | MIT OR Apache-2.0 | https://github.com/Amanieu/parking_lot |
| parking_lot_core | 0.9.12 | MIT OR Apache-2.0 | https://github.com/Amanieu/parking_lot |
| pem | 4.0.0 | MIT | https://github.com/jcreekmore/pem-rs.git |
| percent-encoding | 2.3.2 | MIT OR Apache-2.0 | https://github.com/servo/rust-url/ |
| pin-project-lite | 0.2.18 | Apache-2.0 OR MIT | https://github.com/taiki-e/pin-project-lite |
| png | 0.18.1 | MIT OR Apache-2.0 | https://github.com/image-rs/image-png |
| portable-pty | 0.9.0 | MIT | https://github.com/wezterm/wezterm |
| potential_utf | 0.1.6 | Unicode-3.0 | https://github.com/unicode-org/icu4x |
| powerfmt | 0.2.1 | MIT OR Apache-2.0 | https://github.com/jhpratt/powerfmt |
| ppv-lite86 | 0.2.21 | MIT OR Apache-2.0 | https://github.com/cryptocorrosion/cryptocorrosion |
| proc-macro2 | 1.0.107 | MIT OR Apache-2.0 | https://github.com/dtolnay/proc-macro2 |
| pxfm | 0.1.30 | BSD-3-Clause OR Apache-2.0 | https://github.com/awxkee/pxfm |
| quick-error | 2.0.1 | MIT/Apache-2.0 | http://github.com/tailhook/quick-error |
| quote | 1.0.47 | MIT OR Apache-2.0 | https://github.com/dtolnay/quote |
| rand | 0.10.4 | MIT OR Apache-2.0 | https://github.com/rust-random/rand |
| rand | 0.9.5 | MIT OR Apache-2.0 | https://github.com/rust-random/rand |
| rand_chacha | 0.9.0 | MIT OR Apache-2.0 | https://github.com/rust-random/rand |
| rand_core | 0.10.1 | MIT OR Apache-2.0 | https://github.com/rust-random/rand_core |
| rand_core | 0.6.4 | MIT OR Apache-2.0 | https://github.com/rust-random/rand |
| rand_core | 0.9.5 | MIT OR Apache-2.0 | https://github.com/rust-random/rand |
| rcgen | 0.14.10 | MIT OR Apache-2.0 | https://github.com/rustls/rcgen |
| regex-automata | 0.4.19 | MIT OR Apache-2.0 | https://github.com/rust-lang/regex |
| regex-syntax | 0.8.12 | MIT OR Apache-2.0 | https://github.com/rust-lang/regex |
| reqwest | 0.13.5 | MIT OR Apache-2.0 | https://github.com/seanmonstar/reqwest |
| ring | 0.17.14 | Apache-2.0 AND ISC | https://github.com/briansmith/ring |
| rust-embed | 8.14.0 | MIT | https://pyrossh.dev/repos/rust-embed |
| rust-embed-impl | 8.14.0 | MIT | https://pyrossh.dev/repos/rust-embed |
| rust-embed-utils | 8.14.0 | MIT | https://pyrossh.dev/repos/rust-embed |
| rustc_version | 0.4.1 | MIT OR Apache-2.0 | https://github.com/djc/rustc-version-rs |
| rusticata-macros | 4.1.0 | MIT/Apache-2.0 | https://github.com/rusticata/rusticata-macros.git |
| rustls | 0.23.46 | Apache-2.0 OR ISC OR MIT | https://github.com/rustls/rustls |
| rustls-pki-types | 1.16.0 | MIT OR Apache-2.0 | https://github.com/rustls/pki-types |
| rustls-platform-verifier | 0.7.1 | MIT OR Apache-2.0 | https://github.com/rustls/rustls-platform-verifier |
| rustls-webpki | 0.103.17 | ISC | https://github.com/rustls/webpki |
| ryu | 1.0.23 | Apache-2.0 OR BSL-1.0 | https://github.com/dtolnay/ryu |
| same-file | 1.0.6 | Unlicense/MIT | https://github.com/BurntSushi/same-file |
| scopeguard | 1.2.0 | MIT OR Apache-2.0 | https://github.com/bluss/scopeguard |
| semver | 1.0.28 | MIT OR Apache-2.0 | https://github.com/dtolnay/semver |
| serde | 1.0.229 | MIT OR Apache-2.0 | https://github.com/serde-rs/serde |
| serde_core | 1.0.229 | MIT OR Apache-2.0 | https://github.com/serde-rs/serde |
| serde_derive | 1.0.229 | MIT OR Apache-2.0 | https://github.com/serde-rs/serde |
| serde_json | 1.0.154 | MIT OR Apache-2.0 | https://github.com/serde-rs/json |
| serde_path_to_error | 0.1.20 | MIT OR Apache-2.0 | https://github.com/dtolnay/path-to-error |
| serde_urlencoded | 0.7.1 | MIT/Apache-2.0 | https://github.com/nox/serde_urlencoded |
| serial2 | 0.2.39 | BSD-2-Clause OR Apache-2.0 | https://github.com/de-vri-es/serial2-rs |
| sha1 | 0.10.7 | MIT OR Apache-2.0 | https://github.com/RustCrypto/hashes |
| sha1 | 0.11.0 | MIT OR Apache-2.0 | https://github.com/RustCrypto/hashes |
| sha2 | 0.10.9 | MIT OR Apache-2.0 | https://github.com/RustCrypto/hashes |
| sha2 | 0.11.1 | MIT OR Apache-2.0 | https://github.com/RustCrypto/hashes |
| sharded-slab | 0.1.7 | MIT | https://github.com/hawkw/sharded-slab |
| shared_library | 0.1.9 | Apache-2.0/MIT | https://github.com/tomaka/shared_library/ |
| shell-words | 1.1.1 | MIT/Apache-2.0 | https://github.com/tmiasko/shell-words |
| shlex | 2.0.1 | MIT OR Apache-2.0 | https://github.com/comex/rust-shlex |
| simd-adler32 | 0.3.10 | MIT | https://github.com/mcountryman/simd-adler32 |
| slab | 0.4.12 | MIT | https://github.com/tokio-rs/slab |
| smallvec | 1.16.6 | MIT OR Apache-2.0 | https://github.com/smallvec/smallvec |
| socket-pktinfo | 0.4.2 | MIT | https://github.com/pixsper/socket-pktinfo |
| socket2 | 0.6.5 | MIT OR Apache-2.0 | https://github.com/rust-lang/socket2 |
| spake2 | 0.4.0 | MIT OR Apache-2.0 | https://github.com/RustCrypto/PAKEs/tree/master/spake2 |
| spin | 0.9.9 | MIT | https://github.com/mvdnes/spin-rs.git |
| stable_deref_trait | 1.2.1 | MIT OR Apache-2.0 | https://github.com/storyyeller/stable_deref_trait |
| subtle | 2.6.1 | BSD-3-Clause | https://github.com/dalek-cryptography/subtle |
| syn | 2.0.119 | MIT OR Apache-2.0 | https://github.com/dtolnay/syn |
| syn | 3.0.10 | MIT OR Apache-2.0 | https://github.com/dtolnay/syn |
| sync_wrapper | 1.0.2 | Apache-2.0 | https://github.com/Actyx/sync_wrapper |
| synstructure | 0.13.2 | MIT | https://github.com/mystor/synstructure |
| synstructure | 0.14.0 | MIT | https://github.com/mystor/synstructure |
| sysinfo | 0.40.4 | MIT | https://github.com/GuillaumeGomez/sysinfo |
| thiserror | 1.0.69 | MIT OR Apache-2.0 | https://github.com/dtolnay/thiserror |
| thiserror | 2.0.21 | MIT OR Apache-2.0 | https://github.com/dtolnay/thiserror |
| thiserror-impl | 1.0.69 | MIT OR Apache-2.0 | https://github.com/dtolnay/thiserror |
| thiserror-impl | 2.0.21 | MIT OR Apache-2.0 | https://github.com/dtolnay/thiserror |
| thread_local | 1.1.10 | MIT OR Apache-2.0 | https://github.com/Amanieu/thread_local-rs |
| time | 0.3.55 | MIT OR Apache-2.0 | https://github.com/time-rs/time |
| time-core | 0.1.9 | MIT OR Apache-2.0 | https://github.com/time-rs/time |
| time-macros | 0.2.32 | MIT OR Apache-2.0 | https://github.com/time-rs/time |
| tinystr | 0.8.5 | Unicode-3.0 | https://github.com/unicode-org/icu4x |
| tokio | 1.54.1 | MIT | https://github.com/tokio-rs/tokio |
| tokio-macros | 2.7.2 | MIT | https://github.com/tokio-rs/tokio |
| tokio-rustls | 0.26.6 | MIT OR Apache-2.0 | https://github.com/rustls/tokio-rustls |
| tokio-tungstenite | 0.29.0 | MIT | https://github.com/snapview/tokio-tungstenite |
| tokio-tungstenite | 0.30.0 | MIT | https://github.com/snapview/tokio-tungstenite |
| tokio-util | 0.7.20 | MIT | https://github.com/tokio-rs/tokio |
| tower | 0.5.3 | MIT | https://github.com/tower-rs/tower |
| tower-http | 0.6.11 | MIT | https://github.com/tower-rs/tower-http |
| tower-http | 0.7.1 | MIT | https://github.com/tower-rs/tower-http |
| tower-layer | 0.3.3 | MIT | https://github.com/tower-rs/tower |
| tower-service | 0.3.3 | MIT | https://github.com/tower-rs/tower |
| tracing | 0.1.44 | MIT | https://github.com/tokio-rs/tracing |
| tracing-attributes | 0.1.31 | MIT | https://github.com/tokio-rs/tracing |
| tracing-core | 0.1.36 | MIT | https://github.com/tokio-rs/tracing |
| tracing-log | 0.2.0 | MIT | https://github.com/tokio-rs/tracing |
| tracing-subscriber | 0.3.23 | MIT | https://github.com/tokio-rs/tracing |
| try-lock | 0.2.5 | MIT | https://github.com/seanmonstar/try-lock |
| tungstenite | 0.29.0 | MIT OR Apache-2.0 | https://github.com/snapview/tungstenite-rs |
| tungstenite | 0.30.0 | MIT OR Apache-2.0 | https://github.com/snapview/tungstenite-rs |
| typenum | 1.20.1 | MIT OR Apache-2.0 | https://github.com/paholg/typenum |
| unicase | 2.10.0 | MIT OR Apache-2.0 | https://github.com/seanmonstar/unicase |
| unicode-ident | 1.0.27 | (MIT OR Apache-2.0) AND Unicode-3.0 | https://github.com/dtolnay/unicode-ident |
| untrusted | 0.9.0 | ISC | https://github.com/briansmith/untrusted |
| url | 2.5.8 | MIT OR Apache-2.0 | https://github.com/servo/rust-url |
| utf8_iter | 1.1.0 | Apache-2.0 OR MIT | https://github.com/hsivonen/utf8_iter |
| utf_types | 1.0.0 | Apache-2.0 OR MIT | https://github.com/hsivonen/utf_types |
| version_check | 0.9.5 | MIT/Apache-2.0 | https://github.com/SergioBenitez/version_check |
| walkdir | 2.5.0 | Unlicense/MIT | https://github.com/BurntSushi/walkdir |
| want | 0.3.2 | MIT | https://github.com/seanmonstar/want |
| widestring | 1.2.1 | MIT OR Apache-2.0 | https://github.com/VoidStarKat/widestring-rs |
| winapi | 0.3.9 | MIT/Apache-2.0 | https://github.com/retep998/winapi-rs |
| winapi-util | 0.1.11 | Unlicense OR MIT | https://github.com/BurntSushi/winapi-util |
| windows | 0.61.3 | MIT OR Apache-2.0 | https://github.com/microsoft/windows-rs |
| windows | 0.62.2 | MIT OR Apache-2.0 | https://github.com/microsoft/windows-rs |
| windows-collections | 0.2.0 | MIT OR Apache-2.0 | https://github.com/microsoft/windows-rs |
| windows-collections | 0.3.2 | MIT OR Apache-2.0 | https://github.com/microsoft/windows-rs |
| windows-core | 0.61.2 | MIT OR Apache-2.0 | https://github.com/microsoft/windows-rs |
| windows-core | 0.62.2 | MIT OR Apache-2.0 | https://github.com/microsoft/windows-rs |
| windows-future | 0.2.1 | MIT OR Apache-2.0 | https://github.com/microsoft/windows-rs |
| windows-future | 0.3.2 | MIT OR Apache-2.0 | https://github.com/microsoft/windows-rs |
| windows-implement | 0.60.2 | MIT OR Apache-2.0 | https://github.com/microsoft/windows-rs |
| windows-interface | 0.59.3 | MIT OR Apache-2.0 | https://github.com/microsoft/windows-rs |
| windows-link | 0.1.3 | MIT OR Apache-2.0 | https://github.com/microsoft/windows-rs |
| windows-link | 0.2.1 | MIT OR Apache-2.0 | https://github.com/microsoft/windows-rs |
| windows-numerics | 0.2.0 | MIT OR Apache-2.0 | https://github.com/microsoft/windows-rs |
| windows-numerics | 0.3.1 | MIT OR Apache-2.0 | https://github.com/microsoft/windows-rs |
| windows-result | 0.3.4 | MIT OR Apache-2.0 | https://github.com/microsoft/windows-rs |
| windows-result | 0.4.1 | MIT OR Apache-2.0 | https://github.com/microsoft/windows-rs |
| windows-service | 0.8.1 | MIT OR Apache-2.0 | https://github.com/mullvad/windows-service-rs |
| windows-strings | 0.4.2 | MIT OR Apache-2.0 | https://github.com/microsoft/windows-rs |
| windows-strings | 0.5.1 | MIT OR Apache-2.0 | https://github.com/microsoft/windows-rs |
| windows-sys | 0.61.2 | MIT OR Apache-2.0 | https://github.com/microsoft/windows-rs |
| windows-threading | 0.1.0 | MIT OR Apache-2.0 | https://github.com/microsoft/windows-rs |
| windows-threading | 0.2.1 | MIT OR Apache-2.0 | https://github.com/microsoft/windows-rs |
| winreg | 0.10.1 | MIT | https://github.com/gentoo90/winreg-rs |
| writeable | 0.6.4 | Unicode-3.0 | https://github.com/unicode-org/icu4x |
| x509-parser | 0.18.1 | MIT OR Apache-2.0 | https://github.com/rusticata/x509-parser.git |
| xcap | 0.9.10 | Apache-2.0 | https://github.com/nashaofu/xcap.git |
| yasna | 0.6.0 | MIT OR Apache-2.0 | https://github.com/qnighy/yasna.rs |
| yoke | 0.8.4 | Unicode-3.0 | https://github.com/unicode-org/icu4x |
| yoke-derive | 0.8.4 | Unicode-3.0 | https://github.com/unicode-org/icu4x |
| zerocopy | 0.8.63 | BSD-2-Clause OR Apache-2.0 OR MIT | https://github.com/google/zerocopy |
| zerofrom | 0.1.8 | Unicode-3.0 | https://github.com/unicode-org/icu4x |
| zerofrom-derive | 0.1.8 | Unicode-3.0 | https://github.com/unicode-org/icu4x |
| zeroize | 1.9.2 | Apache-2.0 OR MIT | https://github.com/RustCrypto/utils |
| zerotrie | 0.2.5 | Unicode-3.0 | https://github.com/unicode-org/icu4x |
| zerovec | 0.11.9 | Unicode-3.0 | https://github.com/unicode-org/icu4x |
| zerovec-derive | 0.11.7 | Unicode-3.0 | https://github.com/unicode-org/icu4x |
| zlib-rs | 0.6.8 | Zlib | https://github.com/trifectatechfoundation/zlib-rs |
| zmij | 1.0.23 | MIT | https://github.com/dtolnay/zmij |
| zune-core | 0.5.3 | MIT OR Apache-2.0 OR Zlib | https://github.com/etemesi254/zune-image |
| zune-jpeg | 0.5.15 | MIT OR Apache-2.0 OR Zlib | https://github.com/etemesi254/zune-image/tree/dev/crates/zune-jpeg |

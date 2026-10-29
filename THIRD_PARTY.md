# Сторонние компоненты

Что входит в сборки Braid и на каких условиях. Точные версии — в `core/Cargo.lock`,
`app/rust/Cargo.lock`, `engines/awg-go/go.sum` и `app/pubspec.lock`; полные тексты
лицензий — в исходниках компонентов (`~/.cargo/registry`, `~/go/pkg/mod`, `~/.pub-cache`,
`engines/`). Главное ограничение — `csqtt-core`: PolyForm Noncommercial, поэтому сборки
Braid с движком CSQTT — только для себя и своих, без продажи.

## Движки и платформенный код

| Компонент | Где | Лицензия | Примечание |
|---|---|---|---|
| amneziawg-go v3.1.20260828 (© WireGuard LLC, Amnezia) | `engines/awg-go` → `libawg` (cgo c-shared) | MIT | ядро AmneziaWG как библиотека с узким C-API; форк не нужен — правила приходят подпиской, ключи не перевыпускаются |
| csqtt-core 2.1.9 (форк FOCSQ luminescq/focsq, `feat/macos` a485014; ядро © amurcanov, порт © luminescq, macOS-порт © alextixru) | `engines/csqtt-core` → `core/braid-engine-csqtt` | **PolyForm Noncommercial 1.0.0** | действует на всё, что его линкует: сборки Braid с CSQTT — только некоммерческое использование, для себя и своих. Код PWDTT не включён. Правки под `[BRAID]`, источник и список — `engines/csqtt-core/BRAID-PROVENANCE.md` |
| primp 1.3.1 (deedy5, обрезанная вендорная копия из FOCSQ; `primp-*`-форки hyper/rustls — из crates.io) | `engines/csqtt-core/vendor/primp` (патч `[patch.crates-io]` в `core/Cargo.toml` и `app/rust/Cargo.toml`) | MIT | HTTP-клиент с имитацией TLS/HTTP2-отпечатка браузера для VK API ядра CSQTT; в копии только `Cargo.toml` и `src/`, текст лицензии — в оригинале на crates.io |
| csqtt-tun-helper / tun_darwin.rs (FOCSQ) | источник для `core/braid-helper` | PolyForm Noncommercial 1.0.0 | провенанс — в заголовках файлов |
| cargokit (Matej Knopp) | `app/rust_builder/cargokit` | MIT | сборка Rust внутри `flutter build`; в приложение не попадает |
| flutter_rust_bridge_codegen 2.12 | инструмент разработчика (`make codegen`) | MIT | генерирует `app/lib/src/rust` и `app/rust/src/frb_generated.rs`; в приложение не попадает |

## Go-модули libawg (`engines/awg-go/go.mod`)

Только то, что линкуется в `libawg`; тестовые зависимости самого amneziawg-go в сборку не входят.

| Модуль | Лицензия | Зачем |
|---|---|---|
| github.com/amnezia-vpn/amneziawg-go/v3 | MIT | ядро AmneziaWG: устройство, пиры, obfuscation-параметры |
| golang.org/x/crypto | BSD-3-Clause | криптография WireGuard (Curve25519, ChaCha20-Poly1305, BLAKE2s) |
| golang.org/x/net | BSD-3-Clause | IPv4/IPv6-утилиты, управление сокетами |
| golang.org/x/sys | BSD-3-Clause | системные вызовы |
| golang.zx2c4.com/wintun | MIT | драйвер Wintun — только для Windows, в сборках macOS и Android не линкуется |

## Крейты Rust (`core/Cargo.lock`, `app/rust/Cargo.lock`)

Все зависимости ядра, движков, помощника, `braid-subd`, `braidctl` и моста `braid_ffi` —
пермиссивные лицензии, кроме `csqtt-core` из таблицы выше. Группами по лицензии (имена
крейтов, без версий; часть крейтов нужна только на одной платформе или только сборочным
скриптам):

| Лицензия | Крейты |
|---|---|
| MIT OR Apache-2.0 (237) | addr2line, aead, aes, aes-gcm, android_log-sys, android_logger, android_system_properties, anstream, anstyle, anstyle-parse, anstyle-query, anstyle-wincon, anyhow, arc-swap, arrayvec, asn1-rs, asn1-rs-derive, asn1-rs-impl, async-compression, atomic, atomic-waker, autocfg, backtrace, base64, bit-vec, bitflags, block-buffer, bumpalo, cc, cfg-if, chacha20, chrono, cipher, clap, clap_builder, clap_derive, clap_lex, cmake, cmov, colorchoice, compression-codecs, compression-core, console_error_panic_hook, const-oid, cookie, cookie_store, core-foundation, core-foundation-sys, cpubits, cpufeatures, crc32fast, critical-section, crossbeam-channel, crossbeam-deque, crossbeam-epoch, crossbeam-queue, crossbeam-utils, crypto-common, ctr, ctutils, curve25519-dalek-derive, dart-sys, der-parser, deranged, digest, directories, dirs-sys, displaydoc, document-features, either, env_filter, equivalent, errno, fdeflate, find-msvc-tools, flate2, form_urlencoded, futures, futures-channel, futures-core, futures-executor, futures-io, futures-macro, futures-sink, futures-task, futures-util, getrandom, ghash, gimli, hashbrown, heck, hermit-abi, hex, hickory-proto, hkdf, hmac, http, httparse, httpdate, humantime, hybrid-array, iana-time-zone, iana-time-zone-haiku, idna, idna_adapter, image, indexmap, inout, ipnet, is_terminal_polyfill, itoa, jni, jni-macros, jni-sys, jni-sys-macros, jobserver, js-sys, lazy_static, libc, litrs, lock_api, log, md-5, minimal-lexical, num-bigint, num-conv, num-integer, num-traits, num_cpus, object, oid-registry, once_cell, once_cell_polyfill, openssl-probe, parking_lot, parking_lot_core, percent-encoding, pin-project-lite, pkg-config, png, polyval, portable-atomic, powerfmt, primp-reqwest, primp-tokio-rustls, proc-macro2, psl-types, publicsuffix, qrcode, quinn, quinn-proto, quinn-udp, quote, rand, rand_core, rand_pcg, rayon, rayon-core, rcgen, regex, regex-automata, regex-lite, regex-syntax, rustc-demangle, rustc-hash, rustc_version, rusticata-macros, rustls-pki-types, rustversion, scopeguard, security-framework, security-framework-sys, semver, serde, serde_core, serde_derive, serde_json, serde_spanned, sha1, sha2, shlex, signal-hook-registry, simd_cesu8, simdutf8, smallvec, socket2, stable_deref_trait, syn, system-configuration, system-configuration-sys, thiserror, thiserror-impl, thread_local, threadpool, time, time-core, time-macros, tokio-rustls, toml, toml_datetime, toml_parser, toml_writer, tracing-android, typenum, universal-hash, url, utf8_iter, utf8parse, uuid, version_check, wasm-bindgen, wasm-bindgen-futures, wasm-bindgen-macro, wasm-bindgen-macro-support, wasm-bindgen-shared, web-sys, web-time, windows-core, windows-implement, windows-interface, windows-link, windows-result, windows-strings, windows-sys, windows-targets, windows_aarch64_gnullvm, windows_aarch64_msvc, windows_i686_gnu, windows_i686_gnullvm, windows_i686_msvc, windows_x86_64_gnu, windows_x86_64_gnullvm, windows_x86_64_msvc, x509-parser, yasna, zeroize, zeroize_derive |
| MIT (55) | build-target, bytes, cfg_aliases, combine, dashmap, data-encoding, delegate-attr, flutter_rust_bridge, flutter_rust_bridge_macros, fs_extra, generic-array, http-body, http-body-util, hyper, hyper-util, libredox, matchers, memoffset, mio, nix, nom, nu-ansi-term, oslog, pem, primp, primp-h2, primp-hyper, primp-hyper-util, redox_syscall, redox_users, schannel, sharded-slab, simd-adler32, slab, strsim, synstructure, tokio, tokio-macros, tokio-util, tower, tower-http, tower-layer, tower-service, tracing, tracing-attributes, tracing-core, tracing-log, tracing-subscriber, try-lock, valuable, want, winnow, wintun, zmij, zstd 0.13 |
| Unicode-3.0 (18, ICU4X — нормализация доменных имён в `url`/`idna`) | icu_collections, icu_locale_core, icu_normalizer, icu_normalizer_data, icu_properties, icu_properties_data, icu_provider, litemap, potential_utf, tinystr, writeable, yoke, yoke-derive, zerofrom, zerofrom-derive, zerotrie, zerovec, zerovec-derive |
| BSD-3-Clause (10) | alloc-no-stdlib, alloc-stdlib, c2rust-bitfields, c2rust-bitfields-derive, curve25519-dalek, subtle, x25519-dalek, zstd 0.14, zstd-safe, zstd-sys |
| Unlicense OR MIT (7) | aho-corasick, byteorder, byteorder-lite, memchr, same-file, walkdir, winapi-util |
| MIT OR Apache-2.0 OR Zlib (6) | bytemuck, lru-slab, miniz_oxide, tinyvec, zune-core, zune-jpeg |
| Apache-2.0 OR ISC OR MIT (4) | primp-hyper-rustls, primp-rustls, rustls, rustls-native-certs |
| ISC (3) | libloading, rustls-webpki, untrusted |
| Apache-2.0 (2) | allo-isolate, sync_wrapper |
| 0BSD OR MIT OR Apache-2.0 (2) | adler, adler2 |
| BSD-2-Clause OR Apache-2.0 OR MIT (2) | zerocopy, zerocopy-derive |
| BSD-3-Clause OR Apache-2.0 (2) | moxcms, pxfm |
| Zlib (2) | foldhash, zlib-rs |
| Apache-2.0 AND ISC | ring — криптопровайдер rustls в ядре (`rustls` с `ring`): части на BoringSSL (ISC/OpenSSL-подобная) |
| ISC AND (Apache-2.0 OR ISC) · ISC AND Apache-2.0 AND MIT AND BSD-3-Clause AND … | aws-lc-rs, aws-lc-sys — криптопровайдер форков rustls в `primp` (VK API ядра CSQTT); AWS-LC собирается из исходников (`cmake`) |
| CDLA-Permissive-2.0 | webpki-roots — корневые сертификаты Mozilla (только для проверки обычных сайтов; подписка Braid проверяется пином, не корнями) |
| MPL-2.0 | option-ext — используется как есть, без изменений (MPL допускает это без раскрытия остального кода) |
| MIT OR Apache-2.0 OR LGPL-2.1-or-later | r-efi — берётся по MIT; только для цели UEFI, в сборки не попадает |
| MIT OR Apache-2.0 OR BSD-1-Clause | fiat-crypto |
| (MIT OR Apache-2.0) AND Unicode-3.0 | unicode-ident |
| Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT | wasi — только для цели wasm, в сборки не попадает |
| BSD-3-Clause AND MIT · BSD-3-Clause OR MIT | brotli, brotli-decompressor |
| CC0-1.0 OR MIT-0 OR Apache-2.0 | dunce |

## Flutter и пакеты Dart (`app/pubspec.yaml`)

| Пакет | Лицензия | Зачем |
|---|---|---|
| Flutter SDK 3.47, flutter_localizations, flutter_test | BSD-3-Clause | каркас приложения, русская локаль, тесты |
| flutter_rust_bridge 2.12 | MIT | мост Dart ↔ Rust |
| ffi | BSD-3-Clause | FFI-утилиты |
| path_provider | BSD-3-Clause | каталоги приложения |
| shared_preferences | BSD-3-Clause | настройки UI |
| flutter_secure_storage | BSD-3-Clause | защищённое хранилище (токен VK) |
| tray_manager | MIT | значок в меню-баре macOS |
| window_manager | MIT | окно macOS: закрытие не завершает приложение |
| launch_at_startup | MIT | автозапуск при входе |
| mobile_scanner | BSD-3-Clause | сканер QR на Android |
| qr_flutter | BSD-3-Clause | показ QR на macOS |
| flutter_local_notifications | BSD-3-Clause | уведомления |
| url_launcher | BSD-3-Clause | ссылки на сборки |
| webview_flutter | BSD-3-Clause | капча и вход VK на Android |
| desktop_webview_window | Apache-2.0 | капча и вход VK на десктопе |
| receive_sharing_intent | Apache-2.0 | домен из share-меню Android |
| file_picker | MIT | экспорт и импорт своих правил |
| package_info_plus | BSD-3-Clause | версия приложения |
| intl | BSD-3-Clause | форматирование дат |
| flutter_lints | BSD-3-Clause | только анализ кода, в приложение не попадает |

Транзитивные пакеты Dart (`app/pubspec.lock`): BSD-3-Clause — 79 (платформенные части
плагинов `*_android`, `*_foundation`, `*_linux`, `*_windows`, `*_web`, `*_platform_interface`;
`collection`, `meta`, `path`, `crypto`, `http`, `jni`, `objective_c`, `win32`, `xdg_directories`
и прочие пакеты Dart/Flutter team); MIT — 12 (`screen_retriever*`, `nativeapi*`, `xml`, `yaml`,
`petitparser`, `build_cli_annotations`); Apache-2.0 — 3 (`clock`, `fake_async`,
`material_color_utilities`); MPL-2.0 — `dbus` (только Linux, в сборки macOS и Android не
попадает, используется без изменений); BSD-2-Clause — `timezone`.

## Эталоны интерфейса

Happ и Amnezia — только как образцы экранов (`reference.md`), код не используется.

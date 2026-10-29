# Сторонние компоненты

Что входит в сборки Braid и на каких условиях. Полные тексты лицензий — в
исходниках компонентов (`~/.cargo/registry`, `~/.pub-cache`, `engines/`).

## Движки и платформенный код

| Компонент | Где | Лицензия | Примечание |
|---|---|---|---|
| amneziawg-go (v3) | `engines/awg-go` → `libawg` | MIT | ядро AmneziaWG как библиотека; форк не нужен |
| csqtt-core 2.1.9 (форк FOCSQ luminescq/focsq, `feat/macos` a485014; ядро © amurcanov, порт © luminescq, macOS-порт © alextixru) | `engines/csqtt-core` → `core/braid-engine-csqtt` | **PolyForm Noncommercial 1.0.0** | действует на всё, что его линкует: сборки Braid с CSQTT — только некоммерческое использование, для себя и своих. Код PWDTT не включён. Правки под `[BRAID]`, источник и список — `engines/csqtt-core/BRAID-PROVENANCE.md` |
| primp 1.3.1 (deedy5, обрезанная вендорная копия из FOCSQ; `primp-reqwest` — из crates.io) | `engines/csqtt-core/vendor/primp` (патч `[patch.crates-io]` в `core/Cargo.toml`) | MIT | HTTP-клиент с имитацией TLS/HTTP2-отпечатка браузера для VK API ядра CSQTT; в копии только `Cargo.toml` и `src/`, текст лицензии — в оригинале на crates.io |
| csqtt-tun-helper / tun_darwin.rs (FOCSQ) | источник для `braid-helper` | PolyForm Noncommercial 1.0.0 | провенанс — в заголовках файлов |
| cargokit | `app/rust_builder/cargokit` | MIT | сборка Rust внутри `flutter build` |

## Flutter и пакеты Dart (`app/pubspec.yaml`)

| Пакет | Лицензия | Зачем |
|---|---|---|
| Flutter SDK, flutter_localizations, flutter_test | BSD-3-Clause | каркас приложения, русская локаль |
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

## Крейты Rust (`core/Cargo.toml`, `app/rust`)

Все — MIT и/или Apache-2.0, кроме отмеченных: tokio, bytes, serde, serde_json,
toml, ipnet, url, uuid, chrono, humantime, base64, hex, flate2, thiserror, anyhow,
tracing, tracing-subscriber, rand, sha2, hickory-proto, rustls (Apache-2.0 / ISC / MIT),
ring (ISC и OpenSSL-подобная), rustls-pki-types, tokio-rustls, webpki-roots (CDLA-Permissive-2.0 / MPL-2.0),
rcgen, hyper, hyper-util, http, http-body-util, socket2, libc, nix, directories,
parking_lot, crossbeam-channel, once_cell, smallvec, clap, qrcode, image, jni,
core-foundation, system-configuration, x25519-dalek (BSD-3-Clause), flutter_rust_bridge.

## Эталоны интерфейса

Happ и Amnezia — только как образцы экранов (`reference.md`), код не используется.

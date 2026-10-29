# ADR 0006. Публичный репозиторий и сборки в GitHub Actions

Статус: принято (2026-10-29).

## Контекст

На машине разработки нет Xcode, Android SDK/NDK и JDK — `.app` и APK локально не собрать.
Лимиты GitHub Actions для приватных репозиториев исчерпаны; для публичных минуты бесплатны.
Решение пользователя: «гитхаб экшн, но репо публичный», «только код; правила и `.autopilot/`
вне git».

## Решение

Репозиторий `alextixru/braid` публичный; в git — только код и документация: ни адресов
хостов, ни имён пользователей, ни списков доменов, ни ключей. `state/` (правила, хосты,
пользователи, секреты), `archive/` и `.autopilot/` — в `.gitignore`; в тестах — синтетические
значения. Две проверки:

- `.github/workflows/check.yml` — на каждый push и PR `make check`: версия одна везде,
  fmt/clippy/тесты Rust (ядро, мост, сквозной тест), кросс-проверка платформенного кода под
  Linux/Windows/Android, Go, Flutter (analyze, format, тесты), тесты вендоренного `csqtt-core`.
- `.github/workflows/build.yml` — на push в `main`, вручную и по тегу `v*`: job macOS
  (`macos-latest`: `make libawg helper macos` → `Braid.app` universal с помощником и `libawg`,
  подпись ad-hoc, zip), job Android (`ubuntu-latest` с NDK и Java: `local.properties`,
  Kotlin unit-тесты, `cargo check` ядра под `aarch64-linux-android`, `make libawg-android apk`),
  по тегу — ещё `make check` через `check.yml` и Release с обоими файлами.

Версия одна на всё — файл `VERSION`: `make check-version` сверяет его с `core/Cargo.toml`
(workspace), `app/rust/Cargo.toml` (`client_version()` моста), `app/pubspec.yaml` и обоими
`Cargo.lock`; `make set-version V=x.y.z` меняет всё разом; `make release` проверяет, что тег
`v<VERSION>` ещё не занят; CI по тегу сверяет тег с `VERSION` (`make check-tag`).

## Последствия

- Подпись только ad-hoc, без нотаризации и App Store: установка через «Открыть» в контекстном
  меню, на Android — APK вручную. Network Extension недоступен без Apple Developer — выбран
  root-помощник (ADR 0001, macOS).
- Публичность кода при лицензии `csqtt-core` PolyForm Noncommercial: сборки с CSQTT — только
  для себя и своих; распространять их как продукт нельзя (`THIRD_PARTY.md`).
- Kotlin unit-тесты и сборка ядра под Android проверяются только в CI — локально нет SDK.
- Правило автопилота «`.autopilot/` коммитится» здесь сознательно нарушено по решению пользователя.

# Braid — одна точка входа для установки, проверок и сборок.
# Компонентные цели (libawg, helper, …) живут в своих каталогах и крейтах;
# этот файл их только вызывает. Версия приложения одна на всё — файл VERSION.

SHELL := /bin/bash
.DEFAULT_GOAL := help

# Flutter лежит вне PATH; cargo — в ~/.cargo/bin. Лишние пути безвредны.
export PATH := $(HOME)/development/flutter/bin:$(HOME)/.cargo/bin:$(PATH)

CORE  := core
APP   := app
FFI   := app/rust
AWG   := engines/awg-go
CSQTT := engines/csqtt-core
DIST  := dist
FRB_VERSION := 2.12.0
# Единственный источник версии. Копии в core/Cargo.toml (workspace), app/rust/Cargo.toml
# (client_version() моста) и app/pubspec.yaml сверяет check-version, меняет set-version.
VERSION := $(strip $(shell cat VERSION 2>/dev/null))
# Члены workspace ядра как список «-p имя». Только по ним идут fmt и clippy:
# `cargo fmt --all` берёт и локальные path-зависимости (вендоренный
# engines/csqtt-core), а форматировать вендоренный код нельзя.
CORE_MEMBERS = $(shell cd $(CORE) && cargo metadata --no-deps --format-version 1 2>/dev/null \
	| python3 -I -c 'import json,sys; print(" ".join("-p " + p["name"] for p in json.load(sys.stdin)["packages"]))')
CROSS_TARGETS := x86_64-apple-darwin aarch64-linux-android x86_64-linux-android x86_64-unknown-linux-gnu x86_64-pc-windows-gnu
# ABI Android — только те, под которые есть libawg.so и цели Rust
# (abiFilters в app/android/app/build.gradle.kts).
ANDROID_PLATFORMS := android-arm64,android-x64
MACOS_APP := $(APP)/build/macos/Build/Products/Release/Braid.app
APK := $(APP)/build/app/outputs/flutter-apk/app-release.apk

help: ## список целей
	@grep -hE '^[a-zA-Z_-]+:.*## ' $(MAKEFILE_LIST) | awk 'BEGIN {FS = ":.*## "} {printf "  %-16s %s\n", $$1, $$2}'

# ---------- установка ----------

setup: setup-toolchain setup-targets setup-codegen setup-flutter setup-go ## всё для локальной работы

setup-toolchain: ## rustfmt и clippy для stable
	rustup component add rustfmt clippy

setup-targets: ## цели Rust для кросс-проверок и сборок
	rustup target add $(CROSS_TARGETS)

setup-codegen: ## flutter_rust_bridge_codegen той же версии, что пакет в pubspec
	cargo install flutter_rust_bridge_codegen --version $(FRB_VERSION) --locked

setup-flutter: ## зависимости Flutter
	cd $(APP) && flutter pub get

setup-go: ## зависимости Go-модуля libawg
	cd $(AWG) && go mod download

# ---------- проверки ----------

check: check-version check-rust check-cross check-go check-flutter check-csqtt ## полная проверка — то, что гоняет CI

check-version: ## версия одна: VERSION = core/Cargo.toml = app/rust/Cargo.toml = app/pubspec.yaml (+ Cargo.lock)
	@test -n "$(VERSION)" || { echo "VERSION: файла нет или он пуст"; exit 1; }
	@echo "$(VERSION)" | grep -Eq '^[0-9]+\.[0-9]+\.[0-9]+$$' || { echo "VERSION: «$(VERSION)» — не вида x.y.z"; exit 1; }
	@fail=0; \
	core=$$(awk '/^\[workspace\.package\]/{p=1; next} /^\[/{p=0} p && /^version *=/{gsub(/[" ]/, "", $$3); print $$3}' $(CORE)/Cargo.toml); \
	ffi=$$(awk '/^\[package\]/{p=1; next} /^\[/{p=0} p && /^version *=/{gsub(/[" ]/, "", $$3); print $$3}' $(FFI)/Cargo.toml); \
	pub=$$(sed -n 's/^version: *\([0-9][0-9.]*\).*/\1/p' $(APP)/pubspec.yaml); \
	core_lock=$$(awk '/^name = "braid-core"$$/{p=1; next} p && /^version =/{gsub(/"/, "", $$3); print $$3; exit}' $(CORE)/Cargo.lock); \
	ffi_lock=$$(awk '/^name = "braid_ffi"$$/{p=1; next} p && /^version =/{gsub(/"/, "", $$3); print $$3; exit}' $(FFI)/Cargo.lock); \
	for pair in "core/Cargo.toml:$$core" "app/rust/Cargo.toml:$$ffi" "app/pubspec.yaml:$$pub" "core/Cargo.lock:$$core_lock" "app/rust/Cargo.lock:$$ffi_lock"; do \
	  if [ "$${pair#*:}" != "$(VERSION)" ]; then echo "$${pair%%:*}: $${pair#*:} ≠ VERSION $(VERSION)"; fail=1; fi; \
	done; \
	test $$fail = 0 || { echo "версии разошлись — выровняй: make set-version V=$(VERSION)"; exit 1; }
	@echo "версия $(VERSION) везде одна"

check-tag: ## CI по тегу: TAG=vX.Y.Z совпадает с VERSION
	@test -n "$(TAG)" || { echo "использование: make check-tag TAG=v1.2.3"; exit 1; }
	@test "$(TAG)" = "v$(VERSION)" || { echo "тег $(TAG) ≠ v$(VERSION) из VERSION"; exit 1; }
	@echo "тег $(TAG) соответствует VERSION"

check-tag-free: ## тег v$(VERSION) ещё не существует — локально и в origin
	@! git rev-parse -q --verify "refs/tags/v$(VERSION)" >/dev/null || { echo "тег v$(VERSION) уже есть локально — подними VERSION"; exit 1; }
	@if git remote get-url origin >/dev/null 2>&1; then \
	  if git ls-remote --exit-code --tags origin "refs/tags/v$(VERSION)" >/dev/null 2>&1; then \
	    echo "тег v$(VERSION) уже есть в origin — подними VERSION"; exit 1; fi; \
	else echo "origin не настроен — теги в удалённом репозитории не проверялись"; fi
	@echo "тег v$(VERSION) свободен"

check-rust: ## fmt + clippy по членам workspace ядра, тесты ядра и крейта моста
	@test -n "$(CORE_MEMBERS)" || { echo "cargo metadata не дал членов workspace в $(CORE)"; exit 1; }
	cd $(CORE) && cargo fmt $(CORE_MEMBERS) --check && cargo clippy $(CORE_MEMBERS) --all-targets -- -D warnings && cargo test --workspace
	cd $(FFI) && rustfmt --edition 2021 --check src/lib.rs src/api/*.rs \
		&& CARGO_TARGET_DIR=../../$(CORE)/target cargo clippy --all-targets -- -D warnings \
		&& CARGO_TARGET_DIR=../../$(CORE)/target cargo test

check-cross: ## платформенный код собирается под Linux, Windows и Android (G01: заложено сейчас, собрать после)
	cd $(CORE) && for t in x86_64-unknown-linux-gnu x86_64-pc-windows-gnu aarch64-linux-android; do \
		cargo check -p braid-platform --target $$t || exit 1; done

check-go: ## go vet + тесты libawg
	cd $(AWG) && go vet ./... && go test ./...

check-flutter: ## анализ, формат и тесты приложения; перед тестами собирается мост — иначе ffi_bridge_test пропускается
	cd $(APP) && flutter analyze && dart format --output=none --set-exit-if-changed lib test
	cd $(FFI) && CARGO_TARGET_DIR=../../$(CORE)/target cargo build
	cd $(APP) && flutter test

check-csqtt: ## тесты вендоренного ядра CSQTT — оно не член workspace, поэтому отдельной строкой
	cd $(CSQTT) && cargo test --lib --locked

fmt: ## отформатировать Rust (только члены workspace) и Dart
	@test -n "$(CORE_MEMBERS)" || { echo "cargo metadata не дал членов workspace в $(CORE)"; exit 1; }
	cd $(CORE) && cargo fmt $(CORE_MEMBERS)
	cd $(FFI) && rustfmt --edition 2021 src/lib.rs src/api/*.rs
	cd $(APP) && dart format lib test

codegen: ## перегенерировать мост flutter_rust_bridge (файлы — в git)
	cd $(APP) && flutter_rust_bridge_codegen generate

# ---------- версия ----------

set-version: ## поменять версию везде: make set-version V=x.y.z (VERSION, оба Cargo.toml и Cargo.lock, pubspec)
	@test -n "$(V)" || { echo "использование: make set-version V=x.y.z"; exit 1; }
	@echo "$(V)" | grep -Eq '^[0-9]+\.[0-9]+\.[0-9]+$$' || { echo "V=$(V) — не вида x.y.z"; exit 1; }
	echo "$(V)" > VERSION
	perl -0pi -e 's/(\[workspace\.package\]\s*\nversion = ")[^"]*"/$${1}$(V)"/' $(CORE)/Cargo.toml
	perl -0pi -e 's/(\[package\]\s*\n(?:[^\[\n]*\n)*?version = ")[^"]*"/$${1}$(V)"/' $(FFI)/Cargo.toml
	perl -pi -e 's/^version: *[0-9][0-9.]*(\+\d+)?/version: $(V)$$1/' $(APP)/pubspec.yaml
	cd $(CORE) && cargo update --workspace --offline
	cd $(FFI) && cargo update --workspace --offline
	$(MAKE) check-version

# ---------- сборки ----------

libawg: ## libawg.dylib (universal) и awg.h → engines/awg-go/dist/darwin
	$(MAKE) -C $(AWG) dist

libawg-android: ## libawg.so для arm64-v8a и x86_64 (нужен ANDROID_NDK_HOME)
	$(MAKE) -C $(AWG) dist-android

helper: ## root-помощник macOS
	cd $(CORE) && cargo build -p braid-helper --release

subd: ## сервер подписок и телеметрии
	cd $(CORE) && cargo build -p braid-subd --release

ctl: ## CLI админа
	cd $(CORE) && cargo build -p braidctl --release

macos: libawg ## Braid.app universal с помощником и libawg, ad-hoc подпись, zip в dist/ — нужен Xcode (CI)
	cd $(APP) && flutter build macos --release
# Ad-hoc подпись: сначала вложенное (помощник, libawg), потом бандль целиком; затем проверка.
	codesign --force --sign - "$(MACOS_APP)/Contents/MacOS/braid-helper"
	codesign --force --sign - "$(MACOS_APP)/Contents/Frameworks/libawg.dylib"
	codesign --force --deep --sign - "$(MACOS_APP)"
	codesign --verify --deep --strict "$(MACOS_APP)"
# Universal: обе архитектуры у приложения, помощника и libawg.
	for bin in "$(MACOS_APP)/Contents/MacOS/Braid" "$(MACOS_APP)/Contents/MacOS/braid-helper" "$(MACOS_APP)/Contents/Frameworks/libawg.dylib"; do \
	  archs=$$(lipo -archs "$$bin"); \
	  case "$$archs" in *arm64*x86_64*|*x86_64*arm64*) ;; *) echo "$$bin: не universal ($$archs)"; exit 1;; esac; \
	done
	mkdir -p $(DIST) && rm -f $(DIST)/Braid-$(VERSION)-macos-universal.zip
	ditto -c -k --keepParent "$(MACOS_APP)" $(DIST)/Braid-$(VERSION)-macos-universal.zip
	@ls -l $(DIST)/Braid-$(VERSION)-macos-universal.zip

apk: libawg-android ## APK (arm64-v8a и x86_64) в dist/ — нужны NDK и Java (CI)
	cd $(APP) && flutter build apk --release --target-platform $(ANDROID_PLATFORMS)
	mkdir -p $(DIST) && cp -f $(APK) $(DIST)/Braid-$(VERSION)-android.apk
	@ls -l $(DIST)/Braid-$(VERSION)-android.apk

release: check-version check-tag-free macos apk ## артефакты выпуска в dist/: версия сверена, тег v$(VERSION) свободен
	@ls -l $(DIST)/Braid-$(VERSION)-*
	@echo "Готово. Выпуск: git tag v$(VERSION) && git push origin v$(VERSION) — CI соберёт заново и выложит Release"

clean: ## убрать сборки
	cd $(CORE) && cargo clean
	cd $(APP) && flutter clean
	rm -rf $(DIST)

.PHONY: help setup setup-toolchain setup-targets setup-codegen setup-flutter setup-go \
	check check-version check-tag check-tag-free check-rust check-cross check-go check-flutter check-csqtt \
	fmt codegen set-version libawg libawg-android helper subd ctl macos apk release clean

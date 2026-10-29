# Braid — одна точка входа для установки, проверок и сборок.
# Компонентные цели (libawg, helper, …) живут в своих каталогах и крейтах;
# этот файл их только вызывает.

SHELL := /bin/bash
.DEFAULT_GOAL := help

# Flutter лежит вне PATH; cargo — в ~/.cargo/bin. Лишние пути безвредны.
export PATH := $(HOME)/development/flutter/bin:$(HOME)/.cargo/bin:$(PATH)

CORE := core
APP  := app
FFI  := app/rust
AWG  := engines/awg-go
FRB_VERSION := 2.12.0
# Члены workspace ядра как список «-p имя». Только по ним идут fmt и clippy:
# `cargo fmt --all` берёт и локальные path-зависимости (вендоренный
# engines/csqtt-core), а форматировать вендоренный код нельзя.
CORE_MEMBERS = $(shell cd $(CORE) && cargo metadata --no-deps --format-version 1 2>/dev/null \
	| python3 -I -c 'import json,sys; print(" ".join("-p " + p["name"] for p in json.load(sys.stdin)["packages"]))')
CROSS_TARGETS := x86_64-apple-darwin aarch64-linux-android x86_64-linux-android x86_64-unknown-linux-gnu x86_64-pc-windows-gnu

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

setup-go: ## зависимости Go-модуля libawg (когда он появится)
	@if [ -f $(AWG)/go.mod ]; then cd $(AWG) && go mod download; else echo "$(AWG): модуля ещё нет — пропуск"; fi

# ---------- проверки ----------

check: check-rust check-cross check-go check-flutter ## полная проверка — то, что гоняет CI

check-rust: ## fmt + clippy по членам workspace ядра, тесты ядра и крейта моста
	@test -n "$(CORE_MEMBERS)" || { echo "cargo metadata не дал членов workspace в $(CORE)"; exit 1; }
	cd $(CORE) && cargo fmt $(CORE_MEMBERS) --check && cargo clippy $(CORE_MEMBERS) --all-targets -- -D warnings && cargo test --workspace
	cd $(FFI) && rustfmt --edition 2021 --check src/lib.rs src/api/*.rs \
		&& CARGO_TARGET_DIR=../../$(CORE)/target cargo clippy --all-targets -- -D warnings \
		&& CARGO_TARGET_DIR=../../$(CORE)/target cargo test

check-cross: ## заглушки платформ собираются под Linux, Windows и Android
	cd $(CORE) && for t in x86_64-unknown-linux-gnu x86_64-pc-windows-gnu aarch64-linux-android; do \
		cargo check -p braid-platform --target $$t || exit 1; done

check-go: ## go vet + тесты libawg (когда модуль появится)
	@if [ -f $(AWG)/go.mod ]; then cd $(AWG) && go vet ./... && go test ./...; else echo "$(AWG): модуля ещё нет — пропуск"; fi

check-flutter: ## анализ и тесты приложения
	cd $(APP) && flutter analyze && flutter test

fmt: ## отформатировать Rust (только члены workspace) и Dart
	@test -n "$(CORE_MEMBERS)" || { echo "cargo metadata не дал членов workspace в $(CORE)"; exit 1; }
	cd $(CORE) && cargo fmt $(CORE_MEMBERS)
	cd $(FFI) && rustfmt --edition 2021 src/lib.rs src/api/*.rs
	cd $(APP) && dart format lib test

codegen: ## перегенерировать мост flutter_rust_bridge (файлы — в git)
	cd $(APP) && flutter_rust_bridge_codegen generate

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

macos: ## Braid.app — нужен Xcode (CI)
	cd $(APP) && flutter build macos --release

apk: ## APK — нужны NDK и Java (CI)
	cd $(APP) && flutter build apk --release

release: macos apk ## все артефакты выпуска

clean: ## убрать сборки
	cd $(CORE) && cargo clean
	cd $(APP) && flutter clean

.PHONY: help setup setup-toolchain setup-targets setup-codegen setup-flutter setup-go \
	check check-rust check-cross check-go check-flutter fmt codegen \
	libawg libawg-android helper subd ctl macos apk release clean

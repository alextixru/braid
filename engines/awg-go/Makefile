# libawg — cgo c-shared над amneziawg-go/v3. Вызывается корневым Makefile:
#   make libawg         → dist (darwin universal arm64+x86_64, awg.h)
#   make libawg-android → dist-android (arm64-v8a и x86_64, нужен ANDROID_NDK_HOME)
# dist-linux собирает библиотеку хоста для тестов Rust на Linux (CI).
# Каталог dist/ — вне git.

GO ?= go
DIST := dist
SRC := $(filter-out %_test.go,$(wildcard *.go)) go.mod go.sum awg.h
BUILD := -trimpath -buildmode=c-shared
LDFLAGS := -s -w

.PHONY: dist dist-android dist-linux ndk-check clean

# ---------- macOS ----------

dist: $(DIST)/darwin/libawg.dylib $(DIST)/darwin/awg.h

$(DIST)/darwin/libawg-%.dylib: $(SRC)
	@mkdir -p $(@D)
	CGO_ENABLED=1 GOOS=darwin GOARCH=$* $(GO) build $(BUILD) -ldflags="$(LDFLAGS)" -o $@ .
	@rm -f $(basename $@).h

$(DIST)/darwin/libawg.dylib: $(DIST)/darwin/libawg-arm64.dylib $(DIST)/darwin/libawg-amd64.dylib
	lipo -create -output $@ $^
	install_name_tool -id @rpath/libawg.dylib $@
	codesign -s - -f $@

$(DIST)/darwin/awg.h: awg.h
	@mkdir -p $(@D)
	cp awg.h $@

# ---------- Android ----------

NDK_HOST := $(if $(filter Darwin,$(shell uname -s)),darwin-x86_64,linux-x86_64)
NDK_BIN := $(ANDROID_NDK_HOME)/toolchains/llvm/prebuilt/$(NDK_HOST)/bin
ANDROID_API ?= 26
# 16 КБ-страницы обязательны для targetSdk ≥ 35 (notes-awg.md §3).
ANDROID_LDFLAGS := $(LDFLAGS) -extldflags=-Wl,-z,max-page-size=16384

dist-android: ndk-check $(DIST)/android/arm64-v8a/libawg.so $(DIST)/android/x86_64/libawg.so $(DIST)/android/awg.h

ndk-check:
	@test -n "$(ANDROID_NDK_HOME)" || { echo "ANDROID_NDK_HOME не задан — dist-android собирает CI"; exit 1; }
	@test -x "$(NDK_BIN)/aarch64-linux-android$(ANDROID_API)-clang" || { echo "нет clang в $(NDK_BIN)"; exit 1; }

$(DIST)/android/arm64-v8a/libawg.so: $(SRC)
	@mkdir -p $(@D)
	CGO_ENABLED=1 GOOS=android GOARCH=arm64 CC=$(NDK_BIN)/aarch64-linux-android$(ANDROID_API)-clang \
		$(GO) build $(BUILD) -ldflags="$(ANDROID_LDFLAGS)" -o $@ .
	@rm -f $(@D)/libawg.h

$(DIST)/android/x86_64/libawg.so: $(SRC)
	@mkdir -p $(@D)
	CGO_ENABLED=1 GOOS=android GOARCH=amd64 CC=$(NDK_BIN)/x86_64-linux-android$(ANDROID_API)-clang \
		$(GO) build $(BUILD) -ldflags="$(ANDROID_LDFLAGS)" -o $@ .
	@rm -f $(@D)/libawg.h

$(DIST)/android/awg.h: awg.h
	@mkdir -p $(@D)
	cp awg.h $@

# ---------- Linux (хост CI) ----------

dist-linux: $(DIST)/linux/libawg.so $(DIST)/linux/awg.h

$(DIST)/linux/libawg.so: $(SRC)
	@mkdir -p $(@D)
	CGO_ENABLED=1 GOOS=linux $(GO) build $(BUILD) -ldflags="$(LDFLAGS)" -o $@ .
	@rm -f $(@D)/libawg.h

$(DIST)/linux/awg.h: awg.h
	@mkdir -p $(@D)
	cp awg.h $@

clean:
	rm -rf $(DIST)

// Настоящий шов Dart ↔ Rust: сгенерированный мост грузит собранную
// библиотеку `braid_ffi` и гоняет вызовы через FFI. Ядро здесь не строится
// (оно одно на процесс и тянет платформу), поэтому проверяется то, что можно
// без него: версии, готовность, ошибка «ядро не готово» как `BridgeError`,
// кадр «не готово» потоком, правило sudoers только на чтение.
//
// Библиотеку собирает `cargo build` в `app/rust` (либо путь в `BRAID_FFI_LIB`);
// без неё тест пропускается с причиной.

import 'dart:io';

import 'package:braid/bridge/braid_bridge.dart';
import 'package:braid/bridge/rust_bridge.dart';
import 'package:braid/models/core_state.dart';
import 'package:braid/src/rust/frb_generated.dart';
import 'package:flutter_rust_bridge/flutter_rust_bridge_for_generated.dart';
import 'package:flutter_test/flutter_test.dart';

String? findLibrary() {
  final name = Platform.isMacOS ? 'libbraid_ffi.dylib' : 'libbraid_ffi.so';
  final candidates = [
    Platform.environment['BRAID_FFI_LIB'],
    'rust/target/debug/$name',
    '../core/target/debug/$name',
  ];
  for (final path in candidates) {
    if (path != null && path.isNotEmpty && File(path).existsSync()) {
      return File(path).absolute.path;
    }
  }
  return null;
}

void main() {
  final library = findLibrary();
  final skip = library == null
      ? 'нет собранной libbraid_ffi: выполни `cargo build` в app/rust'
      : (Platform.isMacOS || Platform.isLinux
            ? false
            : 'FFI-проверка только на macOS и Linux');

  const bridge = RustBridge();

  setUpAll(() async {
    if (library == null) return;
    await RustLib.init(externalLibrary: ExternalLibrary.open(library));
  });

  tearDownAll(() {
    if (library != null) RustLib.dispose();
  });

  test('версии приходят из Rust, ядра ещё нет', () {
    expect(bridge.clientVersion(), matches(RegExp(r'^\d+\.\d+\.\d+')));
    expect(bridge.coreVersion(), matches(RegExp(r'^\d+\.\d+\.\d+')));
    expect(bridge.coreReady(), isFalse);
  }, skip: skip);

  test('возможности платформы совпадают с ОС сборки', () {
    final caps = bridge.platformCaps();
    expect(caps.perApp, isFalse);
    expect(caps.qrScan, isFalse);
    expect(caps.menuBar, isTrue);
  }, skip: skip);

  test(
    'без ядра команда падает BridgeError вида notReady с русским текстом',
    () async {
      await expectLater(
        bridge.importLink('braid://x'),
        throwsA(
          isA<BridgeError>()
              .having((e) => e.kind, 'kind', BridgeErrorKind.notReady)
              .having((e) => e.message, 'message', contains('Ядро')),
        ),
      );
      await expectLater(
        bridge.connect(),
        throwsA(
          isA<BridgeError>().having(
            (e) => e.kind,
            'kind',
            BridgeErrorKind.notReady,
          ),
        ),
      );
    },
    skip: skip,
  );

  test(
    'поток состояний без ядра: один кадр «не готово», разобранный Dart-ом',
    () async {
      final frame = await bridge.watchState().first.timeout(
        const Duration(seconds: 5),
      );
      expect(frame.ready, isFalse);
      expect(frame.state.session, const SessionIdle());
      expect(frame.state.hasSubscription, isFalse);
    },
    skip: skip,
  );

  test('правило sudoers читается без побочных действий', () async {
    final status = await bridge.privilegeStatus();
    expect(status.supported, Platform.isMacOS);
    // Помощника рядом с тестовым процессом нет — план честно говорит об этом.
    if (Platform.isMacOS) {
      await expectLater(
        bridge.planSudoersInstall(),
        throwsA(
          isA<BridgeError>().having(
            (e) => e.kind,
            'kind',
            BridgeErrorKind.failed,
          ),
        ),
      );
    }
  }, skip: skip);
}

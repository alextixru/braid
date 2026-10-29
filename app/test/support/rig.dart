/// Сборка контроллера из подделок и «пользователь», который отвечает на
/// вопросы приложения заранее заданными ответами.
library;

import 'dart:async';

import 'package:braid/controller/braid_controller.dart';
import 'package:braid/controller/ui_requests.dart';
import 'package:braid/services/platform_info.dart';
import 'package:braid/services/ui_prefs.dart';
import 'package:braid/services/vpn_channel.dart';
import 'package:flutter/foundation.dart';

import 'fake_bridge.dart';
import 'fakes.dart';

const macOS = PlatformInfo(TargetPlatform.macOS);
const android = PlatformInfo(TargetPlatform.android);

class Rig {
  Rig({
    FakeBridge? bridge,
    this.platform = macOS,
    FakeVpn? vpn,
    FakeClipboard? clipboard,
    FakeDeepLinks? links,
    MemoryUiPrefs? prefs,
    FakeRulesFiles? files,
    FakeQrScanner? scanner,
    FakeUrlOpener? urls,
    Duration serviceStartTimeout = const Duration(seconds: 5),
    Duration serviceGrace = const Duration(milliseconds: 30),
    Duration servicePoll = const Duration(milliseconds: 10),
    DateTime? start,
    bool autoAnswer = true,
  }) : bridge =
           bridge ??
           FakeBridge(caps: platform.isAndroid ? androidCaps : desktopCaps),
       vpn = vpn ?? (platform.isAndroid ? FakeVpn() : null),
       clipboard = clipboard ?? FakeClipboard(),
       links = links ?? FakeDeepLinks(),
       prefs = prefs ?? MemoryUiPrefs(),
       files = files ?? FakeRulesFiles(),
       scanner = scanner ?? FakeQrScanner(),
       urls = urls ?? FakeUrlOpener(),
       clock = start ?? DateTime(2026, 10, 29, 12) {
    controller = BraidController(
      bridge: this.bridge,
      platform: platform,
      vpn: this.vpn ?? const NoVpnChannel(),
      clipboard: this.clipboard,
      links: this.links,
      prefs: this.prefs,
      dirs: const FakeDirs(),
      files: this.files,
      scanner: this.scanner,
      urls: this.urls,
      now: () => clock,
      serviceStartTimeout: serviceStartTimeout,
      serviceGrace: serviceGrace,
      servicePoll: servicePoll,
    );
    // Настоящий сервис по `ACTION_START` сам зовёт `Core::connect`.
    this.vpn?.onStartService ??= () => unawaited(this.bridge.connect());
    // В виджет-тестах вопросы показывает настоящий EventHost, а отвечает тест
    // нажатием кнопки: автоответчик им не нужен.
    if (autoAnswer) _listeners.add(controller.requests.listen(_answer));
    _listeners.add(controller.notices.listen((n) => notices.add(n.message)));
  }

  final FakeBridge bridge;
  final PlatformInfo platform;
  final FakeVpn? vpn;
  final FakeClipboard clipboard;
  final FakeDeepLinks links;
  final MemoryUiPrefs prefs;
  final FakeRulesFiles files;
  final FakeQrScanner scanner;
  final FakeUrlOpener urls;
  late final BraidController controller;

  /// «Часы» приложения: тест двигает их сам.
  DateTime clock;

  // ---------- ответы пользователя ----------

  bool confirmReplace = true;
  bool confirmDomain = true;
  SudoersAnswer sudoersAnswer = SudoersAnswer.install;

  /// Что приложение спросило, в порядке вопросов.
  final asked = <UiRequest<Object?>>[];
  final notices = <String>[];
  final _listeners = <StreamSubscription<Object?>>[];

  void _answer(UiRequest<Object?> request) {
    asked.add(request);
    switch (request) {
      case ConfirmReplace():
        request.complete(confirmReplace);
      case ConfirmDomain():
        request.complete(confirmDomain);
      case OfferSudoers():
        request.complete(sudoersAnswer);
    }
  }

  Iterable<T> askedOf<T extends UiRequest<Object?>>() => asked.whereType<T>();

  Future<void> start() => controller.start();

  Future<void> dispose() async {
    for (final sub in _listeners) {
      await sub.cancel();
    }
    controller.dispose();
    await bridge.close();
  }
}

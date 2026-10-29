import 'package:braid/app.dart';
import 'package:braid/bridge/rust_bridge.dart';
import 'package:braid/controller/braid_controller.dart';
import 'package:braid/screens/onboarding/qr_scanner_screen.dart';
import 'package:braid/services/app_dirs.dart';
import 'package:braid/services/clipboard_access.dart';
import 'package:braid/services/deep_links.dart';
import 'package:braid/services/platform_info.dart';
import 'package:braid/services/qr_scanner.dart';
import 'package:braid/services/rules_files.dart';
import 'package:braid/services/ui_prefs.dart';
import 'package:braid/services/vpn_channel.dart';
import 'package:braid/src/rust/frb_generated.dart';
import 'package:flutter/material.dart';

Future<void> main() async {
  WidgetsFlutterBinding.ensureInitialized();
  try {
    await RustLib.init();
  } on Object catch (e) {
    // Библиотека ядра не загрузилась — без неё работать нечем, но окно
    // объясняет причину, а не закрывается молча.
    runApp(StartupFailedApp(message: 'Не удалось загрузить ядро: $e'));
    return;
  }
  final platform = PlatformInfo.current();
  final controller = BraidController(
    bridge: const RustBridge(),
    platform: platform,
    vpn: platform.isAndroid ? const MethodChannelVpn() : const NoVpnChannel(),
    clipboard: const SystemClipboard(),
    // Android разбирает braid:// сам (MainActivity → сервис → ядро); на
    // десктопе ссылку приносит канал родного слоя.
    links: platform.isDesktop ? ChannelDeepLinks() : const NoDeepLinks(),
    prefs: const SharedUiPrefs(),
    dirs: const SystemAppDirs(),
    files: platform.isDesktop ? const PickerRulesFiles() : const NoRulesFiles(),
    scanner: platform.isAndroid ? const CameraQrScanner() : const NoQrScanner(),
  );
  runApp(BraidApp(controller: controller));
}

/// Значок и меню в меню-баре macOS (ист. 56, G05): держит значок в согласии
/// с состоянием ядра и выполняет выбор в меню.
///
/// Значок обновляется не позже чем через [debounce] после кадра ядра — с
/// запасом внутри «≤ 2 с» из спецификации. Кадры идут часто (каждая проба
/// меняет RTT), а значок, подсказка и меню зависят от немногого, поэтому
/// обновляется только то, что действительно изменилось. Закрытие окна окно
/// скрывает, а приложение остаётся в меню-баре. Если пока окно скрыто,
/// ядру нужен ответ пользователя (диалог) или выбор из меню закончился
/// сообщением, окно поднимается: иначе оно ждало бы там, где его никто не
/// видит.
library;

import 'dart:async';

import 'package:braid/controller/braid_controller.dart';
import 'package:braid/controller/ui_requests.dart';
import 'package:braid/models/commands.dart';
import 'package:braid/models/core_state.dart';
import 'package:braid/tray/system_tray.dart';
import 'package:braid/tray/tray_icon_renderer.dart';
import 'package:braid/tray/tray_status.dart';
import 'package:braid/tray/window_control.dart';
import 'package:braid/util/value.dart';

class TrayService {
  TrayService({
    required this.controller,
    required this.tray,
    required this.icons,
    required this.window,
    this.debounce = const Duration(milliseconds: 150),
  });

  final BraidController controller;
  final SystemTray tray;
  final TrayIconRenderer icons;
  final WindowControl window;

  /// Сколько ждать тишины после кадра, прежде чем обновить значок.
  final Duration debounce;

  String? _iconKey;
  String? _tooltip;
  List<TrayMenuItem>? _menu;
  Timer? _timer;
  StreamSubscription<UiRequest<Object?>>? _requests;
  StreamSubscription<Notice>? _notices;

  /// Окно скрыто нашим «закрытием»; вернуть его можно «Открыть окно».
  bool _hidden = false;

  /// Когда последний раз что-то выбрали в меню: сообщения сразу после этого —
  /// ответ на выбор, а не фоновые.
  DateTime? _actionAt;
  bool _started = false;
  bool _applying = false;
  bool _again = false;
  bool _reported = false;
  bool _disposed = false;
  bool _quitting = false;

  /// Подготовить окно, показать значок и следить за ядром.
  Future<void> start() async {
    try {
      await window.prepare(onCloseRequested: _hide);
    } on Object {
      // Значок нужен и без этого; окно просто закроется как обычно.
      controller.showNotice(
        'Закрытие окна завершит приложение: перехватить его не удалось',
      );
    }
    await tray.show(onSelected: _onSelected);
    _started = true;
    controller.addListener(_schedule);
    _requests = controller.requests.listen((_) => _surface());
    _notices = controller.notices.listen((_) {
      if (_recentAction) _surface();
    });
    await _apply();
  }

  void _hide() {
    _hidden = true;
    unawaited(window.hide());
  }

  Future<void> _show() {
    _hidden = false;
    return window.show();
  }

  /// Поднять окно, если оно скрыто нашим «закрытием».
  void _surface() {
    if (_hidden) unawaited(_show());
  }

  bool get _recentAction {
    final at = _actionAt;
    return at != null &&
        controller.now().difference(at) < const Duration(seconds: 30);
  }

  void _schedule() {
    if (!_started || _disposed) return;
    _timer?.cancel();
    _timer = Timer(debounce, () => unawaited(_apply()));
  }

  Future<void> _apply() async {
    if (_disposed) return;
    if (_applying) {
      _again = true;
      return;
    }
    _applying = true;
    try {
      do {
        _again = false;
        final state = controller.state;
        final busy = controller.busy;
        final status = trayStatus(state, busy: busy);
        final menu = trayMenu(state, controller.catalog, busy: busy);
        if (status.iconKey != _iconKey) {
          await _guard(() async {
            await tray.setIcon(await icons.render(status));
            _iconKey = status.iconKey;
          });
        }
        if (status.tooltip != _tooltip) {
          await _guard(() async {
            await tray.setTooltip(status.tooltip);
            _tooltip = status.tooltip;
          });
        }
        if (_menu == null || !deepEquals(menu, _menu)) {
          await _guard(() async {
            await tray.setMenu(menu);
            _menu = menu;
          });
        }
      } while (_again && !_disposed);
    } finally {
      _applying = false;
    }
  }

  /// Сбой родного слоя не должен ронять приложение; сказать о нём достаточно
  /// один раз, повторять при каждом кадре нечего.
  Future<void> _guard(Future<void> Function() step) async {
    try {
      await step();
    } on Object {
      if (!_reported) {
        _reported = true;
        controller.showNotice('Не удалось обновить значок в меню-баре');
      }
    }
  }

  void _onSelected(String id) {
    switch (id) {
      case 'open':
        unawaited(_show());
      case 'quit':
        unawaited(quit());
      case 'toggle':
        _actionAt = controller.now();
        unawaited(controller.toggleConnection());
      case 'engine:auto':
        _actionAt = controller.now();
        unawaited(controller.setEngineMode(const EngineMode.auto()));
      case _ when id.startsWith('engine:'):
        _actionAt = controller.now();
        unawaited(controller.setEngineMode(EngineMode.pinned(id.substring(7))));
      case _ when id.startsWith('mode:'):
        _actionAt = controller.now();
        unawaited(_selectMode(id.substring(5)));
    }
  }

  Future<void> _selectMode(String id) async {
    final error = await controller.applyRules(RulesChange.selectMode(id));
    if (error != null) controller.showNotice(error);
  }

  /// «Выход»: отключаемся (туннель и DNS возвращаются как были), убираем
  /// значок и завершаем приложение.
  Future<void> quit() async {
    if (_quitting) return;
    _quitting = true;
    final session = controller.state.session;
    final active = session is! SessionIdle && session is! SessionNeedsLink;
    if (active || controller.busy) await controller.disconnect();
    await dispose();
    await window.quit();
  }

  Future<void> dispose() async {
    if (_disposed) return;
    _disposed = true;
    _timer?.cancel();
    unawaited(_requests?.cancel());
    unawaited(_notices?.cancel());
    if (_started) controller.removeListener(_schedule);
    try {
      await tray.dispose();
    } on Object {
      // Значок уже убран системой.
    }
  }
}

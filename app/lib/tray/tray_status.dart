/// Значок и меню в меню-баре macOS (ист. 56, G05): что показать по состоянию
/// ядра. Глиф — по движку, цвет — по живости; меню — подключение, движок,
/// режим, окно, выход. Решение «что показать» не знает про ОС: рисует и
/// слушает щелчки `SystemTray`.
library;

import 'package:braid/app.dart' show appName;
import 'package:braid/models/core_state.dart';
import 'package:braid/models/results.dart';
import 'package:braid/util/format.dart';
import 'package:braid/util/value.dart';

/// Цвет значка по живости.
enum TrayTone {
  /// Не подключено.
  idle,

  /// Подключаюсь или движок запускается.
  working,

  /// Движок работает.
  ok,

  /// Деградация, движок ждёт пользователя, VPN прерван.
  warning,

  /// Движок не отвечает, нет связи с хостом.
  problem,
}

class TrayStatus with Value {
  const TrayStatus({
    required this.glyph,
    required this.tone,
    required this.headline,
  });

  /// Буква в значке: первая буква вида движка (AWG → A, CSQTT → C); без
  /// движка — B от названия приложения.
  final String glyph;
  final TrayTone tone;

  /// Состояние словами: «Подключено · AWG».
  final String headline;

  String get tooltip => '$appName · $headline';

  /// Один ключ на одинаковый значок: по нему кэшируется картинка и
  /// пропускаются лишние обновления.
  String get iconKey => '$glyph:${tone.name}';

  @override
  List<Object?> get props => [glyph, tone, headline];
}

String _glyph(EngineView? engine) {
  final kind = engine?.kind.trim() ?? '';
  return kind.isEmpty ? appName[0].toUpperCase() : kind[0].toUpperCase();
}

/// Состояние значка. [busy] — подключение запущено, а кадр «подключаюсь» ещё
/// не пришёл (как на главном экране).
TrayStatus trayStatus(CoreState state, {bool busy = false}) {
  final session = state.session;
  final active = state.engineById(state.active);
  if (busy && (session is SessionIdle || session is SessionNeedsLink)) {
    return TrayStatus(
      glyph: _glyph(null),
      tone: TrayTone.working,
      headline: 'Подключаюсь…',
    );
  }
  return switch (session) {
    SessionConnected() => _connected(state, active),
    SessionConnecting() => TrayStatus(
      glyph: _glyph(active),
      tone: TrayTone.working,
      headline: 'Подключаюсь…',
    ),
    SessionNoHost() => TrayStatus(
      glyph: _glyph(active),
      tone: TrayTone.problem,
      headline: 'Нет связи с хостом',
    ),
    SessionInterrupted(:final byOtherVpn) => TrayStatus(
      glyph: _glyph(active),
      tone: TrayTone.warning,
      headline: byOtherVpn
          ? 'VPN перехвачен другим приложением'
          : 'Подключение прервано системой',
    ),
    SessionIdle() || SessionNeedsLink() => TrayStatus(
      glyph: _glyph(null),
      tone: TrayTone.idle,
      headline: 'Отключено',
    ),
  };
}

TrayStatus _connected(CoreState state, EngineView? active) {
  final name = active == null ? null : engineLabel(active, state.engines);
  final (tone, suffix) = switch (active?.state) {
    null || EngineStatus.connected => (TrayTone.ok, null),
    EngineStatus.degraded => (TrayTone.warning, 'деградация'),
    EngineStatus.starting => (TrayTone.working, 'запускается'),
    EngineStatus.waitingUser => (TrayTone.warning, 'ждёт ответа'),
    EngineStatus.dead => (TrayTone.problem, 'не отвечает'),
    EngineStatus.stopped => (TrayTone.problem, 'остановлен'),
  };
  return TrayStatus(
    glyph: _glyph(active),
    tone: tone,
    headline: ['Подключено', ?name, ?suffix].join(' · '),
  );
}

/// Пункт меню в меню-баре. Пустой [id] — разделитель.
class TrayMenuItem with Value {
  const TrayMenuItem({
    required this.id,
    required this.label,
    this.enabled = true,
    this.checked = false,
    this.children = const [],
  });

  const TrayMenuItem.separator()
    : id = '',
      label = '',
      enabled = false,
      checked = false,
      children = const [];

  final String id;
  final String label;
  final bool enabled;

  /// Отмеченный пункт выбора (движок, режим).
  final bool checked;

  /// Подменю.
  final List<TrayMenuItem> children;

  bool get isSeparator => id.isEmpty;

  @override
  List<Object?> get props => [id, label, enabled, checked, children];
}

/// Меню значка: состояние, подключение, «Движок», «Режим», окно, выход.
List<TrayMenuItem> trayMenu(
  CoreState state,
  RulesCatalog catalog, {
  bool busy = false,
}) {
  final status = trayStatus(state, busy: busy);
  final session = state.session;
  final connecting = busy || session is SessionConnecting;
  final toggleLabel = switch (session) {
    _ when connecting => 'Отменить',
    SessionConnected() => 'Отключить',
    SessionNoHost() => 'Повторить',
    SessionInterrupted() => 'Вернуть VPN',
    _ => 'Подключить',
  };
  final pinned = state.engines.where((engine) => engine.pinned);
  return [
    TrayMenuItem(id: 'status', label: status.headline, enabled: false),
    const TrayMenuItem.separator(),
    TrayMenuItem(
      id: 'toggle',
      label: toggleLabel,
      enabled: state.hasSubscription,
    ),
    TrayMenuItem(
      id: 'engine',
      label: 'Движок',
      enabled: state.engines.isNotEmpty,
      children: [
        TrayMenuItem(id: 'engine:auto', label: 'Авто', checked: pinned.isEmpty),
        for (final engine in state.engines)
          TrayMenuItem(
            id: 'engine:${engine.id}',
            label: engineLabel(engine, state.engines),
            checked: engine.pinned,
          ),
      ],
    ),
    TrayMenuItem(
      id: 'mode',
      label: 'Режим',
      enabled: catalog.modes.isNotEmpty,
      children: [
        for (final mode in catalog.modes)
          TrayMenuItem(
            id: 'mode:${mode.id}',
            label: mode.name,
            checked: state.rules.mode == mode.id,
          ),
      ],
    ),
    const TrayMenuItem.separator(),
    const TrayMenuItem(id: 'open', label: 'Открыть окно'),
    const TrayMenuItem(id: 'quit', label: 'Выход'),
  ];
}

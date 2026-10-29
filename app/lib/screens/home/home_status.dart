import 'package:braid/controller/braid_controller.dart';
import 'package:braid/models/core_state.dart';
import 'package:braid/util/format.dart';

/// Как выглядит состояние на главном экране.
enum StatusTone { idle, working, ok, problem, warning }

/// Что делает большая кнопка.
enum ConnectAction { connect, disconnect }

/// Состояние сессии, переведённое в тексты и действия главного экрана
/// (ист. 1, 6, 9): ровно одна главная кнопка на состояние, при необходимости —
/// вторая, чтобы снять защиту совсем.
class HomeStatus {
  const HomeStatus({
    required this.title,
    required this.tone,
    required this.buttonLabel,
    required this.action,
    this.detail,
    this.offerDisconnect = false,
  });

  final String title;
  final String? detail;
  final StatusTone tone;
  final String buttonLabel;
  final ConnectAction action;

  /// Рядом с главной кнопкой «Повторить» нужна ещё «Отключить».
  final bool offerDisconnect;

  factory HomeStatus.of(BraidController controller) {
    final state = controller.state;
    final session = state.session;
    // Подключение запущено, а кадр «подключаюсь» ещё не пришёл (запрос прав,
    // запуск сервиса): показываем это сразу, а не «Отключено».
    if (controller.busy &&
        (session is SessionIdle || session is SessionNeedsLink)) {
      return const HomeStatus(
        title: 'Подключаюсь…',
        tone: StatusTone.working,
        buttonLabel: 'Отменить',
        action: ConnectAction.disconnect,
      );
    }
    switch (session) {
      case SessionConnected():
        final active = state.engineById(state.active);
        final duration = controller.connectedFor;
        return HomeStatus(
          title: [
            'Подключено',
            if (active != null) engineLabel(active, state.engines),
            if (duration != null) formatDuration(duration),
          ].join(' · '),
          tone: StatusTone.ok,
          buttonLabel: 'Отключить',
          action: ConnectAction.disconnect,
        );
      case SessionConnecting():
        return const HomeStatus(
          title: 'Подключаюсь…',
          tone: StatusTone.working,
          buttonLabel: 'Отменить',
          action: ConnectAction.disconnect,
        );
      case SessionNoHost(:final lastError):
        return HomeStatus(
          title: 'Нет связи с хостом',
          detail: prettyReason(lastError, state.engines),
          tone: StatusTone.problem,
          buttonLabel: 'Повторить',
          action: ConnectAction.connect,
          offerDisconnect: true,
        );
      case SessionInterrupted(:final byOtherVpn):
        return HomeStatus(
          title: byOtherVpn
              ? 'VPN перехвачен другим приложением'
              : 'Подключение прервано системой',
          detail: byOtherVpn
              ? 'Другое приложение заняло VPN-интерфейс.'
              : 'Система закрыла интерфейс или отозвала разрешение VPN.',
          tone: StatusTone.warning,
          buttonLabel: 'Вернуть VPN',
          action: ConnectAction.connect,
        );
      case SessionIdle() || SessionNeedsLink():
        final conflicts = state.banners.whereType<OtherVpnBanner>();
        if (conflicts.isNotEmpty) {
          return HomeStatus(
            title: 'Отключи ${conflicts.first.iface}',
            detail: 'Другой VPN держит маршруты и DNS. Отключи его и подключись снова.',
            tone: StatusTone.warning,
            buttonLabel: 'Подключить',
            action: ConnectAction.connect,
          );
        }
        return const HomeStatus(
          title: 'Отключено',
          detail: 'Нажми, чтобы подключиться',
          tone: StatusTone.idle,
          buttonLabel: 'Подключить',
          action: ConnectAction.connect,
        );
    }
  }
}

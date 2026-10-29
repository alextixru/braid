/// Тексты экрана «Движки»: состояние словами, отклик, потери, возраст
/// хендшейка (ист. 14–17, 22, 30, 74).
library;

import 'package:braid/models/core_state.dart';

/// Что с движком, одной строкой.
String engineStatusText(EngineView engine, {required bool active}) {
  return switch (engine.state) {
    EngineStatus.connected => active ? 'Работает · несёт трафик' : 'Работает',
    EngineStatus.starting => 'Запускается…',
    EngineStatus.degraded => 'Связь деградировала',
    // Закреплённый мёртвый движок не подменяется (ист. 30): говорим об этом.
    EngineStatus.dead =>
      engine.pinned
          ? 'Недоступен — закреплён вручную, автопереключения нет'
          : 'Недоступен',
    EngineStatus.stopped => 'Остановлен',
    EngineStatus.waitingUser => switch (engine.userAction) {
      CaptchaAction() => 'Ждёт капчу',
      NeedHashesAction() => 'Нужны новые хеши',
      VkLoginAction() => 'Нужен вход через VK',
      null => 'Ждёт ответа',
    },
  };
}

/// Отклик, потери и возраст хендшейка: «Отклик 38 мс · потери 0 % · хендшейк
/// 12 с назад». У движка без хендшейка (CSQTT) его в строке нет. `null` — о
/// движке ничего не известно.
String? healthText({int? rttMs, required int lossPct, int? handshakeAgeS}) {
  if (rttMs == null && lossPct == 0 && handshakeAgeS == null) return null;
  final parts = [
    if (rttMs != null) 'отклик $rttMs мс',
    'потери $lossPct %',
    if (handshakeAgeS != null) 'хендшейк ${ageText(handshakeAgeS)}',
  ];
  final text = parts.join(' · ');
  return text[0].toUpperCase() + text.substring(1);
}

/// «12 с назад», «3 мин назад», «2 ч назад»; ноль — «только что».
String ageText(int seconds) {
  if (seconds <= 0) return 'только что';
  if (seconds < 60) return '$seconds с назад';
  if (seconds < 3600) return '${seconds ~/ 60} мин назад';
  return '${seconds ~/ 3600} ч назад';
}

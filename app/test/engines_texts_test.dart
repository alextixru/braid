// Что экран «Движки» пишет про каждый движок (ист. 14–17, 22, 30, 74).

import 'package:braid/models/core_state.dart';
import 'package:braid/screens/engines/engine_texts.dart';
import 'package:flutter_test/flutter_test.dart';

EngineView engine(
  EngineStatus state, {
  bool pinned = false,
  UserAction? action,
}) => EngineView(
  id: 'csqtt',
  kind: 'csqtt',
  state: state,
  pinned: pinned,
  userAction: action,
);

void main() {
  group('состояние словами', () {
    test('работает; активный несёт трафик', () {
      expect(
        engineStatusText(engine(EngineStatus.connected), active: false),
        'Работает',
      );
      expect(
        engineStatusText(engine(EngineStatus.connected), active: true),
        'Работает · несёт трафик',
      );
    });

    test('запускается, деградировал, остановлен', () {
      expect(
        engineStatusText(engine(EngineStatus.starting), active: false),
        'Запускается…',
      );
      expect(
        engineStatusText(engine(EngineStatus.degraded), active: true),
        'Связь деградировала',
      );
      expect(
        engineStatusText(engine(EngineStatus.stopped), active: false),
        'Остановлен',
      );
    });

    test('мёртвый закреплённый не подменяется — это видно (ист. 30)', () {
      expect(
        engineStatusText(engine(EngineStatus.dead), active: false),
        'Недоступен',
      );
      expect(
        engineStatusText(
          engine(EngineStatus.dead, pinned: true),
          active: false,
        ),
        'Недоступен — закреплён вручную, автопереключения нет',
      );
    });

    test('ждёт пользователя: капча, новые хеши, вход VK (ист. 16, 17)', () {
      String waiting(UserAction? action) => engineStatusText(
        engine(EngineStatus.waitingUser, action: action),
        active: false,
      );
      expect(
        waiting(const CaptchaAction(url: 'https://x.y', session: 's')),
        'Ждёт капчу',
      );
      expect(waiting(const NeedHashesAction()), 'Нужны новые хеши');
      expect(waiting(const VkLoginAction()), 'Нужен вход через VK');
      expect(waiting(null), 'Ждёт ответа');
    });
  });

  group('здоровье', () {
    test('отклик, потери и возраст хендшейка (ист. 74)', () {
      expect(
        healthText(rttMs: 38, lossPct: 0, handshakeAgeS: 12),
        'Отклик 38 мс · потери 0 % · хендшейк 12 с назад',
      );
    });

    test('нет отклика — строка начинается с потерь', () {
      expect(
        healthText(lossPct: 100, handshakeAgeS: 200),
        'Потери 100 % · хендшейк 3 мин назад',
      );
    });

    test('у CSQTT хендшейка нет — его в строке нет', () {
      expect(healthText(rttMs: 45, lossPct: 3), 'Отклик 45 мс · потери 3 %');
    });

    test('данных нет совсем — строки нет', () {
      expect(healthText(lossPct: 0), isNull);
    });

    test('возраст: секунды, минуты, часы', () {
      expect(ageText(0), 'только что');
      expect(ageText(59), '59 с назад');
      expect(ageText(60), '1 мин назад');
      expect(ageText(200), '3 мин назад');
      expect(ageText(3600), '1 ч назад');
      expect(ageText(7300), '2 ч назад');
    });
  });
}

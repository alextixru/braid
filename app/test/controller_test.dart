// Контроллер приложения на подделках моста и сервисов: запуск, подключение,
// подписка, правила, настройки. Диалоги здесь — запросы, на которые отвечает
// «пользователь» из `Rig`; виджетов нет.

import 'dart:async';

import 'package:braid/bridge/braid_bridge.dart';
import 'package:braid/controller/connect_models.dart';
import 'package:braid/controller/ui_requests.dart';
import 'package:braid/models/commands.dart';
import 'package:braid/models/core_state.dart';
import 'package:braid/models/results.dart';
import 'package:braid/models/settings.dart';
import 'package:braid/services/ui_prefs.dart';
import 'package:flutter_test/flutter_test.dart';

import 'support/fake_bridge.dart';
import 'support/fakes.dart';
import 'support/rig.dart';
import 'support/states.dart';

BridgeError failure(
  BridgeErrorKind kind,
  String message, [
  List<String> details = const [],
]) => BridgeError(kind: kind, message: message, details: details);

void main() {
  group('запуск', () {
    test(
      'десктоп: ядро строится в каталоге приложения, всё нужное читается',
      () async {
        final rig = Rig();
        addTearDown(rig.dispose);
        await rig.start();

        expect(rig.bridge.startedWith, '/tmp/braid-test-core');
        expect(
          rig.bridge.calls,
          containsAll(['startCore', 'watchState', 'settings', 'rulesCatalog']),
        );
        expect(rig.controller.phase, StartPhase.ready);
        expect(rig.controller.coreReady, isTrue);
        expect(rig.controller.state, idleState());
        expect(rig.controller.catalog, catalogFixture);
        expect(rig.controller.caps, desktopCaps);
      },
    );

    test(
      'сессию после перезагрузки восстанавливает ядро — окно resume() не зовёт',
      () async {
        final rig = Rig();
        addTearDown(rig.dispose);
        await rig.start();
        await pumpEventQueue();
        expect(rig.bridge.calls, isNot(contains('resume')));
      },
    );

    test('Android: ядра ещё нет — просим сервис только поднять ядро, не подключаться; окно ждёт готовности', () async {
      final bridge = FakeBridge(ready: false, caps: androidCaps);
      final vpn = FakeVpn(onEnsureCore: () => bridge.emit(isReady: true));
      final rig = Rig(bridge: bridge, platform: android, vpn: vpn);
      addTearDown(rig.dispose);
      await rig.start();

      expect(vpn.calls, contains('ensureCore'));
      expect(
        vpn.calls,
        isNot(contains('startService')),
        reason:
            'startService — это «подключить»: окно зовёт его только по кнопке',
      );
      expect(rig.controller.phase, StartPhase.ready);
      expect(rig.controller.coreReady, isTrue);
      expect(
        bridge.startedWith,
        isNull,
        reason: 'на Android ядро строит сервис, не мост',
      );
      expect(
        bridge.calls,
        isNot(contains('resume')),
        reason: 'восстановление — дело сервиса',
      );
    });

    test('Android: сервис уже поднимает ядро (стартовал при загрузке) — второй запуск не нужен', () async {
      final bridge = FakeBridge(ready: false, caps: androidCaps);
      final vpn = FakeVpn()..running = true;
      final rig = Rig(bridge: bridge, platform: android, vpn: vpn);
      addTearDown(rig.dispose);
      final starting = rig.start();
      await pumpEventQueue();
      expect(rig.controller.phase, StartPhase.starting);

      bridge.emit(isReady: true);
      await starting;
      expect(vpn.calls, isNot(contains('ensureCore')));
      expect(vpn.calls, isNot(contains('startService')));
      expect(rig.controller.phase, StartPhase.ready);
    });

    test('Android: ядро уже есть — сервис повторно не запускается', () async {
      final bridge = FakeBridge(caps: androidCaps);
      final vpn = FakeVpn();
      final rig = Rig(bridge: bridge, platform: android, vpn: vpn);
      addTearDown(rig.dispose);
      await rig.start();
      expect(vpn.calls, isNot(contains('ensureCore')));
      expect(vpn.calls, isNot(contains('startService')));
      expect(rig.controller.phase, StartPhase.ready);
    });

    test(
      'Android: сервис не построил ядро — ошибка запуска, а не вечное ожидание',
      () async {
        final bridge = FakeBridge(ready: false, caps: androidCaps);
        final rig = Rig(
          bridge: bridge,
          platform: android,
          vpn: FakeVpn(),
          serviceStartTimeout: const Duration(milliseconds: 30),
        );
        addTearDown(rig.dispose);
        await rig.start();

        expect(rig.controller.phase, StartPhase.failed);
        expect(rig.controller.startError, contains('Сервис'));
      },
    );

    test(
      'ядро не построилось — ошибка запуска с текстом ядра, повтор возможен',
      () async {
        final bridge = FakeBridge()
          ..startFailure = failure(
            BridgeErrorKind.failed,
            'Не открыть хранилище',
          );
        final rig = Rig(bridge: bridge);
        addTearDown(rig.dispose);
        await rig.start();
        expect(rig.controller.phase, StartPhase.failed);
        expect(rig.controller.startError, 'Не открыть хранилище');

        bridge.startFailure = null;
        await rig.start();
        expect(rig.controller.phase, StartPhase.ready);
        expect(rig.controller.startError, isNull);
      },
    );

    test('ссылка, которой открыли приложение, импортируется сразу', () async {
      final bridge = FakeBridge(state: noSubscriptionState)..link = null;
      final rig = Rig(
        bridge: bridge,
        links: FakeDeepLinks(initial: 'braid://first'),
      );
      addTearDown(rig.dispose);
      await rig.start();
      await pumpEventQueue();
      expect(bridge.importedLinks, ['braid://first']);
      expect(rig.controller.state.hasSubscription, isTrue);
    });
  });

  group('импорт ссылки', () {
    test('первая ссылка загружает подписку', () async {
      final bridge = FakeBridge(state: noSubscriptionState)..link = null;
      final rig = Rig(bridge: bridge);
      addTearDown(rig.dispose);
      await rig.start();

      final result = await rig.controller.importLink('braid://abc');
      expect(result, isA<ImportDone>());
      expect(bridge.importedLinks, ['braid://abc']);
      await pumpEventQueue();
      expect(rig.controller.state.hasSubscription, isTrue);
    });

    test(
      '«не ссылка Braid» возвращается текстом ядра, подписка не тронута',
      () async {
        final bridge = FakeBridge(state: noSubscriptionState)..link = null;
        final rig = Rig(bridge: bridge);
        addTearDown(rig.dispose);
        await rig.start();

        final result = await rig.controller.importLink('привет');
        expect(
          result,
          isA<ImportFailed>().having(
            (r) => r.message,
            'message',
            'Это не ссылка Braid',
          ),
        );
        expect(rig.controller.state.hasSubscription, isFalse);
      },
    );

    test(
      'пробелы и переводы строк по краям вставленной ссылки отбрасываются',
      () async {
        final bridge = FakeBridge(state: noSubscriptionState)..link = null;
        final rig = Rig(bridge: bridge);
        addTearDown(rig.dispose);
        await rig.start();
        await rig.controller.importLink('  braid://abc\n');
        expect(bridge.importedLinks, ['braid://abc']);
      },
    );

    test('пустой ввод не доходит до ядра', () async {
      final rig = Rig(bridge: FakeBridge(state: noSubscriptionState));
      addTearDown(rig.dispose);
      await rig.start();
      final result = await rig.controller.importLink('   ');
      expect(result, isA<ImportFailed>());
      expect(rig.bridge.importedLinks, isEmpty);
    });

    test(
      'ссылка другого хоста спрашивает замену; согласие меняет подписку',
      () async {
        final rig = Rig();
        addTearDown(rig.dispose);
        await rig.start();

        final result = await rig.controller.importLink('braid://other');
        expect(result, isA<ImportDone>());
        final question = rig.askedOf<ConfirmReplace>().single;
        expect(
          (question.currentHost, question.newHost),
          ('Франкфурт', 'new.example'),
        );
        expect(rig.bridge.replacedLinks, ['braid://other']);
        await pumpEventQueue();
        expect(rig.controller.state.subscription.hostName, 'Хельсинки');
      },
    );

    test('отказ от замены оставляет прежнюю подписку', () async {
      final rig = Rig()..confirmReplace = false;
      addTearDown(rig.dispose);
      await rig.start();

      final result = await rig.controller.importLink('braid://other');
      expect(result, isA<ImportCancelled>());
      expect(rig.bridge.replacedLinks, isEmpty);
      expect(rig.controller.state.subscription.hostName, 'Франкфурт');
    });

    test(
      'ссылка, пришедшая в работающее приложение, идёт тем же путём',
      () async {
        final rig = Rig();
        addTearDown(rig.dispose);
        await rig.start();

        rig.links.open('braid://other');
        await pumpEventQueue();
        expect(rig.askedOf<ConfirmReplace>(), hasLength(1));
        expect(rig.bridge.replacedLinks, ['braid://other']);
      },
    );

    test('ссылка, которую сервис Android не принял без замены, спрашивается при возврате', () async {
      final vpn = FakeVpn()..pendingLink = 'braid://other';
      final rig = Rig(
        bridge: FakeBridge(caps: androidCaps),
        platform: android,
        vpn: vpn,
      );
      addTearDown(rig.dispose);
      await rig.start();
      await pumpEventQueue();
      expect(rig.askedOf<ConfirmReplace>(), hasLength(1));
      expect(rig.bridge.replacedLinks, ['braid://other']);
    });
  });

  group('подключение на macOS', () {
    PrivilegeStatus status({bool rule = false, bool current = false}) =>
        PrivilegeStatus(
          supported: true,
          ruleInstalled: rule,
          helperCurrent: current,
        );

    test('первое подключение предлагает не спрашивать пароль; согласие ставит показанную команду', () async {
      final bridge = FakeBridge(privilege: status());
      final rig = Rig(bridge: bridge);
      addTearDown(rig.dispose);
      await rig.start();

      await rig.controller.connect();
      await pumpEventQueue();

      final offer = rig.askedOf<OfferSudoers>().single;
      expect(offer.plan, sudoersPlanFixture);
      expect(offer.update, isFalse);
      expect(bridge.appliedSudoers, [sudoersPlanFixture.command]);
      expect(
        bridge.calls.indexOf('applySudoersInstall'),
        lessThan(bridge.calls.indexOf('connect')),
      );
      expect(rig.controller.privilege!.ruleInstalled, isTrue);
      expect(rig.controller.state.session, const SessionConnected());
    });

    test('«Не сейчас» запоминается: подключение идёт, предложение больше не повторяется', () async {
      final bridge = FakeBridge(privilege: status());
      final rig = Rig(bridge: bridge)..sudoersAnswer = SudoersAnswer.later;
      addTearDown(rig.dispose);
      await rig.start();

      await rig.controller.connect();
      expect(bridge.appliedSudoers, isEmpty);
      expect(bridge.calls, contains('connect'));
      expect(rig.prefs.sudoersDeclined, isTrue);

      await rig.controller.disconnect();
      await rig.controller.connect();
      expect(rig.askedOf<OfferSudoers>(), hasLength(1));
    });

    test('правило уже стоит и помощник тот же — предлагать нечего', () async {
      final bridge = FakeBridge(privilege: status(rule: true, current: true));
      final rig = Rig(bridge: bridge);
      addTearDown(rig.dispose);
      await rig.start();
      await rig.controller.connect();
      expect(rig.askedOf<OfferSudoers>(), isEmpty);
      expect(bridge.calls, isNot(contains('planSudoersInstall')));
    });

    test('платформа без sudoers (не macOS) правило не предлагает', () async {
      final rig = Rig(bridge: FakeBridge());
      addTearDown(rig.dispose);
      await rig.start();
      await rig.controller.connect();
      expect(rig.askedOf<OfferSudoers>(), isEmpty);
    });

    test(
      'сбой установки правила не мешает подключиться; причина видна',
      () async {
        final bridge = FakeBridge(privilege: status())
          ..sudoersError = failure(BridgeErrorKind.denied, 'Пароль не принят');
        final rig = Rig(bridge: bridge);
        addTearDown(rig.dispose);
        await rig.start();

        await rig.controller.connect();
        expect(rig.notices, contains('Пароль не принят'));
        expect(bridge.calls, contains('connect'));
      },
    );

    test('после обновления приложения правило устарело: флаг есть, предложение при подключении — нет', () async {
      final bridge = FakeBridge(privilege: status(rule: true, current: false));
      final rig = Rig(bridge: bridge);
      addTearDown(rig.dispose);
      await rig.start();
      expect(rig.controller.ruleNeedsUpdate, isTrue);

      await rig.controller.connect();
      expect(
        rig.askedOf<OfferSudoers>(),
        isEmpty,
        reason: 'об этом говорит баннер, а не подключение',
      );
    });

    test(
      '«Обновить правило» показывает ту же процедуру с пометкой об обновлении',
      () async {
        final bridge = FakeBridge(
          privilege: status(rule: true, current: false),
        );
        final rig = Rig(bridge: bridge);
        addTearDown(rig.dispose);
        await rig.start();

        await rig.controller.installSudoersRule();
        final offer = rig.askedOf<OfferSudoers>().single;
        expect(offer.update, isTrue);
        expect(bridge.appliedSudoers, [sudoersPlanFixture.command]);
        expect(rig.controller.ruleNeedsUpdate, isFalse);
      },
    );

    test('конфликт с другим VPN: пользователь видит «Отключи <имя>»', () async {
      final bridge = FakeBridge(privilege: status(rule: true, current: true))
        ..connectError = failure(BridgeErrorKind.conflict, 'Отключи utun5', [
          'utun5',
        ]);
      final rig = Rig(bridge: bridge);
      addTearDown(rig.dispose);
      await rig.start();

      await rig.controller.connect();
      expect(rig.notices, ['Отключи utun5']);
      expect(rig.controller.busy, isFalse);
    });

    test('отказ в пароле администратора объясняется, а не молчит', () async {
      final bridge = FakeBridge(privilege: status(rule: true, current: true))
        ..connectError = failure(
          BridgeErrorKind.denied,
          'Запрос прав администратора отменён',
        );
      final rig = Rig(bridge: bridge);
      addTearDown(rig.dispose);
      await rig.start();
      await rig.controller.connect();
      expect(rig.notices, ['Запрос прав администратора отменён']);
    });

    test(
      'без подписки подключение отправляет к ссылке, а не показывает сбой',
      () async {
        final bridge = FakeBridge(state: noSubscriptionState)
          ..connectError = failure(
            BridgeErrorKind.needsLink,
            'Сначала вставь ссылку или отсканируй QR',
          );
        final rig = Rig(bridge: bridge);
        addTearDown(rig.dispose);
        await rig.start();
        await rig.controller.connect();
        expect(rig.notices, ['Сначала вставь ссылку или отсканируй QR']);
      },
    );

    test('«Отключить» из другого места (меню-бар), пока открыт вопрос о правиле, отменяет подключение', () async {
      final bridge = FakeBridge(privilege: status());
      final rig = Rig(bridge: bridge, autoAnswer: false);
      addTearDown(rig.dispose);
      OfferSudoers? offer;
      final sub = rig.controller.requests.listen((r) {
        if (r is OfferSudoers) offer = r;
      });
      addTearDown(sub.cancel);
      await rig.start();

      final connecting = rig.controller.connect();
      await pumpEventQueue();
      expect(offer, isNotNull);

      await rig.controller
          .toggleConnection(); // пока подключение «busy» — отмена
      offer!.complete(SudoersAnswer.later);
      await connecting;

      expect(bridge.calls, isNot(contains('connect')));
      expect(bridge.calls, contains('disconnect'));
      expect(rig.controller.busy, isFalse);
    });

    test(
      'второй тап во время подключения не запускает второе connect()',
      () async {
        final bridge = FakeBridge(privilege: status(rule: true, current: true))
          ..connectGate = Completer();
        final rig = Rig(bridge: bridge);
        addTearDown(rig.dispose);
        await rig.start();

        final first = rig.controller.connect();
        await pumpEventQueue();
        expect(rig.controller.busy, isTrue);
        await rig.controller.connect();
        expect(bridge.calls.where((c) => c == 'connect'), hasLength(1));

        bridge.connectGate!.complete();
        await first;
        expect(rig.controller.busy, isFalse);
      },
    );
  });

  group('подключение на Android', () {
    Rig androidRig({FakeVpn? vpn, FakeBridge? bridge, Duration? timeout}) =>
        Rig(
          bridge: bridge ?? FakeBridge(caps: androidCaps),
          platform: android,
          vpn: vpn ?? FakeVpn(),
          serviceStartTimeout: timeout ?? const Duration(seconds: 5),
        );

    test(
      'без согласия VPN сервис не стартует и ядро не просят подключаться',
      () async {
        final vpn = FakeVpn(consent: false);
        final rig = androidRig(vpn: vpn);
        addTearDown(rig.dispose);
        await rig.start();

        await rig.controller.connect();
        expect(vpn.calls.where((c) => c == 'prepare' || c == 'startService'), [
          'prepare',
        ]);
        expect(rig.bridge.calls, isNot(contains('connect')));
        expect(rig.notices, ['Без разрешения VPN подключиться нельзя']);
      },
    );

    test(
      'сервис не запущен: окно запускает его, а подключает сам сервис',
      () async {
        final vpn = FakeVpn();
        final rig = androidRig(vpn: vpn);
        addTearDown(rig.dispose);
        await rig.start();

        await rig.controller.connect();
        await pumpEventQueue();
        expect(vpn.calls, containsAllInOrder(['prepare', 'startService']));
        expect(
          rig.bridge.calls.where((c) => c == 'connect'),
          hasLength(1),
          reason:
              'connect() — от сервиса; ещё один из окна гонялся бы с привязкой '
              'платформы к новому экземпляру сервиса',
        );
        expect(rig.controller.state.session, const SessionConnected());
        expect(rig.notices, isEmpty);
      },
    );

    test(
      'сервис уже работает и привязан к ядру — окно подключает напрямую',
      () async {
        final vpn = FakeVpn()..running = true;
        final rig = androidRig(vpn: vpn);
        addTearDown(rig.dispose);
        await rig.start();

        await rig.controller.connect();
        await pumpEventQueue();
        expect(vpn.calls, isNot(contains('startService')));
        expect(rig.bridge.calls, contains('connect'));
        expect(rig.controller.state.session, const SessionConnected());
      },
    );

    test(
      'сервис уже работает, а ядро отказало — ошибка ядра видна в окне',
      () async {
        final bridge = FakeBridge(caps: androidCaps)
          ..connectError = failure(
            BridgeErrorKind.denied,
            'VPN не разрешён системой',
          );
        final rig = androidRig(vpn: FakeVpn()..running = true, bridge: bridge);
        addTearDown(rig.dispose);
        await rig.start();
        await rig.controller.connect();
        expect(rig.notices, ['VPN не разрешён системой']);
      },
    );

    test('сервис остановился, не начав сессию — окно отсылает к уведомлению, кнопка свободна', () async {
      final vpn = FakeVpn();
      vpn.onStartService = () =>
          vpn.running = false; // сервис сам вышел после ошибки
      final rig = androidRig(vpn: vpn);
      addTearDown(rig.dispose);
      await rig.start();

      final timer = Stopwatch()..start();
      await rig.controller.connect();
      timer.stop();
      expect(rig.notices, [
        'Не удалось подключиться. Подробности — в уведомлении Braid.',
      ]);
      expect(rig.bridge.calls, isNot(contains('connect')));
      expect(rig.controller.busy, isFalse);
      expect(
        timer.elapsed,
        lessThan(const Duration(seconds: 2)),
        reason: 'сервис остановился — не ждём полный таймаут',
      );
    });

    test('отмена, пока ждём сессию, не оставляет ложной ошибки', () async {
      final vpn = FakeVpn();
      vpn.onStartService = () {}; // сервис жив, но сессии пока нет
      final rig = androidRig(vpn: vpn);
      addTearDown(rig.dispose);
      await rig.start();

      final connecting = rig.controller.connect();
      await pumpEventQueue();
      expect(rig.controller.busy, isTrue);
      await rig.controller.toggleConnection(); // пока busy — это отмена
      await connecting;

      expect(rig.notices, isEmpty);
      expect(rig.controller.busy, isFalse);
      expect(vpn.calls.last, 'stopService');
    });

    test('отключение останавливает и сервис', () async {
      final vpn = FakeVpn();
      final rig = androidRig(vpn: vpn);
      addTearDown(rig.dispose);
      await rig.start();
      await rig.controller.connect();
      await rig.controller.disconnect();

      expect(rig.bridge.calls, containsAllInOrder(['connect', 'disconnect']));
      expect(vpn.calls.last, 'stopService');
    });

    test('при возврате в приложение домен из share-меню спрашивается и добавляется', () async {
      final vpn = FakeVpn();
      final rig = androidRig(vpn: vpn);
      addTearDown(rig.dispose);
      await rig.start();
      await pumpEventQueue();

      // Домен пришёл из share-меню, пока окно было в фоне.
      vpn.sharedDomain = 'site.com';
      await rig.controller.onAppResumed();
      expect(rig.askedOf<ConfirmDomain>().single.domain, 'site.com');
      expect(rig.bridge.rulesChanges, [
        const RulesChange.addUserDomain('site.com'),
      ]);
      expect(vpn.sharedDomain, isNull);
    });

    test('ядро ещё не готово (сервис убит системой): connect ждёт его и не гонится с сервисом', () async {
      final bridge = FakeBridge(caps: androidCaps);
      final vpn = FakeVpn();
      final rig = androidRig(vpn: vpn, bridge: bridge);
      addTearDown(rig.dispose);
      await rig.start();

      bridge.emit(isReady: false);
      await pumpEventQueue();
      vpn.onStartService = () =>
          Future<void>.delayed(const Duration(milliseconds: 20), () {
            bridge.emit(isReady: true);
            unawaited(bridge.connect());
          });

      await rig.controller.connect();
      expect(vpn.calls, containsAllInOrder(['prepare', 'startService']));
      expect(bridge.calls.where((c) => c == 'connect'), hasLength(1));
      expect(rig.notices, isEmpty);
    });

    test(
      'сервис так и не построил ядро — сообщение, подключение не стартует',
      () async {
        final bridge = FakeBridge(caps: androidCaps);
        final rig = androidRig(
          bridge: bridge,
          vpn: FakeVpn()..onStartService = () {},
          timeout: const Duration(milliseconds: 40),
        );
        addTearDown(rig.dispose);
        await rig.start();
        bridge.emit(isReady: false);
        await pumpEventQueue();

        await rig.controller.connect();
        expect(rig.notices, [
          'Сервис не запустился. Закрой приложение и открой снова.',
        ]);
        expect(bridge.calls, isNot(contains('connect')));
        expect(rig.controller.busy, isFalse);
      },
    );
  });

  group('кнопка подключения', () {
    test('подключено → отключает; не подключено → подключает; «нет связи» → повторяет', () async {
      final rig = Rig(bridge: FakeBridge(state: connectedState()));
      addTearDown(rig.dispose);
      await rig.start();
      await rig.controller.toggleConnection();
      expect(rig.bridge.calls.last, 'disconnect');
      await pumpEventQueue();

      await rig.controller.toggleConnection();
      expect(rig.bridge.calls.last, 'connect');

      rig.bridge.emit(
        next: idleState(session: const SessionNoHost('Проба не ответила')),
      );
      await pumpEventQueue();
      await rig.controller.toggleConnection();
      expect(rig.bridge.calls.last, 'connect');
    });

    test('«подключаюсь» — та же кнопка отменяет', () async {
      final rig = Rig(
        bridge: FakeBridge(
          state: idleState(session: const SessionConnecting()),
        ),
      );
      addTearDown(rig.dispose);
      await rig.start();
      await rig.controller.toggleConnection();
      expect(rig.bridge.calls.last, 'disconnect');
    });

    test('длительность считается от перехода в «подключено»', () async {
      final rig = Rig();
      addTearDown(rig.dispose);
      await rig.start();
      expect(rig.controller.connectedFor, isNull);

      rig.bridge.emit(next: idleState(session: const SessionConnecting()));
      await pumpEventQueue();
      rig.bridge.emit(next: connectedState());
      await pumpEventQueue();
      expect(rig.controller.connectedFor, Duration.zero);

      rig.clock = rig.clock.add(const Duration(minutes: 12));
      expect(rig.controller.connectedFor, const Duration(minutes: 12));

      rig.bridge.emit(next: idleState());
      await pumpEventQueue();
      expect(rig.controller.connectedFor, isNull);
    });

    test('приложение открыли на уже идущем подключении — длительность честно неизвестна', () async {
      final rig = Rig(bridge: FakeBridge(state: connectedState()));
      addTearDown(rig.dispose);
      await rig.start();
      expect(rig.controller.connectedFor, isNull);
    });
  });

  group('правила', () {
    test('ошибка ядра возвращается текстом, остальное не ломается', () async {
      final bridge = FakeBridge()
        ..rulesError = failure(BridgeErrorKind.invalidInput, 'Это не домен');
      final rig = Rig(bridge: bridge);
      addTearDown(rig.dispose);
      await rig.start();
      expect(
        await rig.controller.applyRules(const RulesChange.addUserDomain('x')),
        'Это не домен',
      );
      expect(
        await rig.controller.applyRules(const RulesChange.addUserDomain('x.y')),
        isNull,
      );
    });

    test(
      '«Мои правила»: ссылка становится доменом, адрес — подсетью',
      () async {
        final rig = Rig();
        addTearDown(rig.dispose);
        await rig.start();

        expect(await rig.controller.addRule('https://x.y/путь'), isNull);
        expect(await rig.controller.addRule(' 10.1.2.3/8 '), isNull);
        expect(rig.bridge.rulesChanges, [
          const RulesChange.addUserDomain('x.y'),
          const RulesChange.addUserCidr('10.0.0.0/8'),
        ]);
      },
    );

    test('«Мои правила»: мусор не доходит до ядра, ошибка объяснена', () async {
      final rig = Rig();
      addTearDown(rig.dispose);
      await rig.start();
      final error = await rig.controller.addRule('привет, мир');
      expect(error, isNotNull);
      expect(rig.bridge.rulesChanges, isEmpty);
    });

    test(
      '«+ Домен»: из буфера берётся домен и спрашивается подтверждение',
      () async {
        final rig = Rig(
          clipboard: FakeClipboard('https://www.site.com/page?utm=1'),
        );
        addTearDown(rig.dispose);
        await rig.start();

        await rig.controller.addDomainFromClipboard();
        expect(rig.askedOf<ConfirmDomain>().single.domain, 'site.com');
        expect(rig.bridge.rulesChanges, [
          const RulesChange.addUserDomain('site.com'),
        ]);
        expect(rig.notices, ['Добавлено: site.com']);
      },
    );

    test('«+ Домен»: отказ в диалоге ничего не добавляет', () async {
      final rig = Rig(clipboard: FakeClipboard('site.com'))
        ..confirmDomain = false;
      addTearDown(rig.dispose);
      await rig.start();
      await rig.controller.addDomainFromClipboard();
      expect(rig.bridge.rulesChanges, isEmpty);
    });

    test('«+ Домен»: в буфере нет домена — подсказка вместо диалога', () async {
      final rig = Rig(clipboard: FakeClipboard('привет'));
      addTearDown(rig.dispose);
      await rig.start();
      await rig.controller.addDomainFromClipboard();
      expect(rig.asked, isEmpty);
      expect(rig.notices, ['В буфере нет ссылки или домена']);
    });

    test('«+ Домен»: уже добавленный домен не спрашивается повторно', () async {
      final state = idleState(
        rules: const RulesView(
          sets: [],
          mode: 'work',
          userDomains: ['site.com'],
        ),
      );
      final rig = Rig(
        bridge: FakeBridge(state: state),
        clipboard: FakeClipboard('site.com'),
      );
      addTearDown(rig.dispose);
      await rig.start();
      await rig.controller.addDomainFromClipboard();
      expect(rig.asked, isEmpty);
      expect(rig.notices, ['site.com уже в твоих правилах']);
    });

    test('закрепление движка и возврат в «Авто» уходят в ядро', () async {
      final rig = Rig();
      addTearDown(rig.dispose);
      await rig.start();
      await rig.controller.setEngineMode(const EngineMode.pinned('csqtt'));
      await rig.controller.setEngineMode(const EngineMode.auto());
      expect(rig.bridge.engineModes, [
        const EngineMode.pinned('csqtt'),
        const EngineMode.auto(),
      ]);
    });
  });

  group('настройки', () {
    test('изменение пишется в ядро и видно сразу', () async {
      final rig = Rig();
      addTearDown(rig.dispose);
      await rig.start();
      // Kill switch включён по умолчанию — изменение здесь это его выключение.
      await rig.controller.updateSettings((s) => s.copyWith(killSwitch: false));
      expect(rig.controller.settings.killSwitch, isFalse);
      expect(rig.bridge.settingsWrites.single.killSwitch, isFalse);
    });

    test('отказ ядра откатывает настройку и сообщает причину', () async {
      final bridge = FakeBridge()
        ..settingsError = failure(
          BridgeErrorKind.denied,
          'Автозапуск не разрешён системой',
        );
      final rig = Rig(bridge: bridge);
      addTearDown(rig.dispose);
      await rig.start();
      await rig.controller.updateSettings((s) => s.copyWith(autostart: true));
      expect(rig.controller.settings.autostart, isFalse);
      expect(rig.notices, ['Автозапуск не разрешён системой']);
    });

    test('«всегда подключаться здесь» запоминает сеть и подключает', () async {
      final state = idleState(
        network: 'wifi:aa',
        banners: const [AutoconnectAskBanner(network: 'wifi:aa')],
      );
      final rig = Rig(bridge: FakeBridge(state: state));
      addTearDown(rig.dispose);
      await rig.start();

      await rig.controller.answerAutoconnect(
        'wifi:aa',
        AutoconnectAnswer.always,
      );
      expect(rig.controller.settings.autoconnectNetworks, {
        'wifi:aa': Autoconnect.connect,
      });
      expect(rig.bridge.calls, contains('connect'));
    });

    test('«никогда здесь» запоминает сеть и не подключает; вопрос больше не показывается', () async {
      final state = idleState(
        network: 'wifi:aa',
        banners: const [AutoconnectAskBanner(network: 'wifi:aa')],
      );
      final rig = Rig(bridge: FakeBridge(state: state));
      addTearDown(rig.dispose);
      await rig.start();
      expect(
        rig.controller.banners,
        contains(const AutoconnectAskBanner(network: 'wifi:aa')),
      );

      await rig.controller.answerAutoconnect(
        'wifi:aa',
        AutoconnectAnswer.never,
      );
      expect(rig.controller.settings.autoconnectNetworks, {
        'wifi:aa': Autoconnect.never,
      });
      expect(rig.bridge.calls, isNot(contains('connect')));
      expect(rig.controller.banners, isEmpty);
    });

    test('«только сейчас» подключает, правило сети не меняя', () async {
      final state = idleState(
        network: 'wifi:aa',
        banners: const [AutoconnectAskBanner(network: 'wifi:aa')],
      );
      final rig = Rig(bridge: FakeBridge(state: state));
      addTearDown(rig.dispose);
      await rig.start();
      await rig.controller.answerAutoconnect('wifi:aa', AutoconnectAnswer.once);
      expect(rig.controller.settings.autoconnectNetworks, isEmpty);
      expect(rig.bridge.calls, contains('connect'));
    });
  });

  group('подписка и баннеры', () {
    test('каталог правил перечитывается, когда подписка меняется', () async {
      final bridge = FakeBridge();
      final rig = Rig(bridge: bridge);
      addTearDown(rig.dispose);
      await rig.start();
      final before = bridge.calls.where((c) => c == 'rulesCatalog').length;

      const newer = SubscriptionView(
        hostId: 'frankfurt',
        hostName: 'Франкфурт',
        version: 8,
      );
      bridge.emit(next: idleState(subscription: newer));
      await pumpEventQueue();
      expect(bridge.calls.where((c) => c == 'rulesCatalog').length, before + 1);

      // Тот же хост и версия — перечитывать незачем.
      bridge.emit(
        next: idleState(
          session: const SessionConnecting(),
          subscription: newer,
        ),
      );
      await pumpEventQueue();
      expect(bridge.calls.where((c) => c == 'rulesCatalog').length, before + 1);
    });

    test('ручное обновление сообщает итог', () async {
      final bridge = FakeBridge()..refreshOutcome = const Updated(8);
      final rig = Rig(bridge: bridge);
      addTearDown(rig.dispose);
      await rig.start();
      await rig.controller.refreshSubscription();
      expect(rig.notices, ['Подписка обновлена · версия 8']);

      bridge.refreshOutcome = const NotModified();
      await rig.controller.refreshSubscription();
      expect(rig.notices.last, 'Изменений нет — подписка актуальна');
    });

    test('ошибка обновления показывается текстом ядра', () async {
      final bridge = FakeBridge()
        ..refreshError = failure(BridgeErrorKind.failed, 'Хост недоступен');
      final rig = Rig(bridge: bridge);
      addTearDown(rig.dispose);
      await rig.start();
      await rig.controller.refreshSubscription();
      expect(rig.notices, ['Хост недоступен']);
    });

    test(
      'баннер обновления закрывается для версии и возвращается для следующей',
      () async {
        const update = UpdateAvailableBanner(latest: '0.2.0');
        final bridge = FakeBridge(state: idleState(banners: const [update]));
        final rig = Rig(bridge: bridge);
        addTearDown(rig.dispose);
        await rig.start();
        expect(rig.controller.banners, [update]);

        await rig.controller.dismissUpdate('0.2.0');
        expect(rig.controller.banners, isEmpty);
        expect(rig.prefs.updateDismissed, '0.2.0');

        bridge.emit(
          next: idleState(
            banners: const [UpdateAvailableBanner(latest: '0.3.0')],
          ),
        );
        await pumpEventQueue();
        expect(rig.controller.banners, [
          const UpdateAvailableBanner(latest: '0.3.0'),
        ]);
      },
    );

    test('закрытое обновление помнится между запусками', () async {
      final prefs = MemoryUiPrefs()..updateDismissed = '0.2.0';
      final rig = Rig(
        bridge: FakeBridge(
          state: idleState(
            banners: const [UpdateAvailableBanner(latest: '0.2.0')],
          ),
        ),
        prefs: prefs,
      );
      addTearDown(rig.dispose);
      await rig.start();
      expect(rig.controller.banners, isEmpty);
    });

    test('«другой VPN» показывается в статусе, а не плашкой; остальные плашки остаются', () async {
      final state = idleState(
        banners: [
          const OtherVpnBanner(iface: 'utun3'),
          const NeedsHashesBanner(),
          SubscriptionStaleBanner(since: DateTime.utc(2026, 10, 28)),
        ],
      );
      final rig = Rig(bridge: FakeBridge(state: state));
      addTearDown(rig.dispose);
      await rig.start();
      expect(rig.controller.banners, [
        const NeedsHashesBanner(),
        SubscriptionStaleBanner(since: DateTime.utc(2026, 10, 28)),
      ]);
    });

    test('ссылка подписки для QR отдаётся мостом', () async {
      final rig = Rig();
      addTearDown(rig.dispose);
      await rig.start();
      expect(
        await rig.controller.subscriptionLink(),
        'braid://current-subscription',
      );
    });
  });

  group('устойчивость', () {
    test(
      'сбой родного слоя ссылок при запуске не мешает окну, причина видна',
      () async {
        final links = FakeDeepLinks()
          ..initialError = StateError('канал ссылок сломан');
        final rig = Rig(links: links);
        addTearDown(rig.dispose);
        await rig.start();
        await pumpEventQueue();
        expect(rig.controller.phase, StartPhase.ready);
        expect(rig.notices, [
          'Внутренняя ошибка: Bad state: канал ссылок сломан',
        ]);
      },
    );

    test('не прочиталось правило sudoers при возврате в приложение — окно живёт и помнит прежнее', () async {
      final bridge = FakeBridge(
        privilege: const PrivilegeStatus(
          supported: true,
          ruleInstalled: true,
          helperCurrent: true,
        ),
      );
      final rig = Rig(bridge: bridge);
      addTearDown(rig.dispose);
      await rig.start();
      final before = rig.controller.privilege;

      bridge.privilegeError = const BridgeError(
        kind: BridgeErrorKind.failed,
        message: 'нет доступа к каталогу',
        details: [],
      );
      await rig.controller.onAppResumed();
      await pumpEventQueue();
      expect(rig.controller.privilege, before);
      expect(rig.notices, isEmpty);
    });

    test('нечитаемый кадр не роняет контроллер; следующий нормальный снимает ошибку', () async {
      final rig = Rig();
      addTearDown(rig.dispose);
      await rig.start();

      rig.bridge.emitBroken('поле «kind»: ожидалась строка');
      await pumpEventQueue();
      expect(rig.controller.protocolError, contains('kind'));
      expect(
        rig.controller.state,
        idleState(),
        reason: 'прежнее состояние не теряется',
      );

      rig.bridge.emit(next: connectedState());
      await pumpEventQueue();
      expect(rig.controller.protocolError, isNull);
      expect(rig.controller.state.session, const SessionConnected());
    });

    test(
      'неожиданное исключение моста становится сообщением, а не падением',
      () async {
        final bridge = FakeBridge();
        final rig = Rig(bridge: bridge);
        addTearDown(rig.dispose);
        await rig.start();
        bridge.importHandler = (_) => throw StateError('внутренний сбой');
        final result = await rig.controller.importLink('braid://x');
        expect(result, isA<ImportFailed>());
      },
    );
  });
}

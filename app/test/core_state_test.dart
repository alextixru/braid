// Шов Rust ↔ Dart: состояние, настройки, каталог правил и команды в
// serde-форме ядра. Документы — те же, что проверяет Rust (`contract.rs`,
// `wire.rs`): если форма разойдётся, красными станут обе стороны.

import 'dart:convert';

import 'package:braid/models/commands.dart';
import 'package:braid/models/core_state.dart';
import 'package:braid/models/results.dart';
import 'package:braid/models/settings.dart';
import 'package:flutter_test/flutter_test.dart';

import 'support/fixtures.dart';

void main() {
  group('кадр состояния', () {
    test('полный эталонный кадр читается без потерь', () {
      final frame = StateFrame.fromJson(fixtureObject('core_state_full.json'));

      expect(frame.ready, isTrue);
      final state = frame.state;
      expect(state.session, const SessionConnected());
      expect(state.active, 'awg');
      expect(state.admin, isTrue);
      expect(state.network, 'wifi:00aa11bb22cc33dd');

      expect(state.engines, hasLength(2));
      final awg = state.engines[0];
      expect(awg.id, 'awg');
      expect(awg.kind, 'awg');
      expect(awg.state, EngineStatus.connected);
      expect(
        awg.health,
        const Health(rttMs: 38, lossPct: 0, handshakeAgeS: 12),
      );
      expect(awg.pinned, isFalse);
      expect(awg.userAction, isNull);
      final csqtt = state.engines[1];
      expect(csqtt.state, EngineStatus.waitingUser);
      expect(csqtt.health, const Health());
      expect(csqtt.pinned, isTrue);
      expect(csqtt.warm, isTrue);
      expect(
        csqtt.userAction,
        const CaptchaAction(
          url: 'https://id.vk.example/captcha',
          session: 's-1',
        ),
      );

      expect(
        state.rules.sets.map(
          (s) => (s.id, s.name, s.enabled, s.domains, s.ips),
        ),
        [('google', 'Google', true, 3, 2), ('cursor', 'Cursor', false, 2, 0)],
      );
      expect(state.rules.mode, 'work');
      expect(state.rules.userDomains, ['example.org']);
      expect(state.rules.userCidrs, ['10.20.0.0/16']);
      expect(state.rules.apps, const AppRulesExcluded(['org.bank.app']));

      final sub = state.subscription;
      expect(sub.hostId, 'frankfurt');
      expect(sub.hostName, 'Франкфурт');
      expect(sub.version, 7);
      expect(sub.updatedAt, DateTime.utc(2026, 10, 29, 10, 15));
      expect(sub.lastRefreshError, 'Хост недоступен');
      expect(sub.revoked, isFalse);
      expect(state.hasSubscription, isTrue);

      expect(state.banners, [
        const UpdateAvailableBanner(
          latest: '0.2.0',
          url: 'https://example.org/Braid.zip',
        ),
        const NeedsHashesBanner(),
        const NeedsVkLoginBanner(),
        SubscriptionStaleBanner(since: DateTime.utc(2026, 10, 28, 8)),
        const RevokedBanner(),
        const OtherVpnBanner(iface: 'utun3'),
        const AutoconnectAskBanner(network: 'wifi:00aa11bb22cc33dd'),
      ]);
    });

    test('каждое состояние сессии из эталона читается как задумано', () {
      final expected = <String, SessionState>{
        'idle': const SessionIdle(),
        'needs_link': const SessionNeedsLink(),
        'connecting': const SessionConnecting(),
        'connected': const SessionConnected(),
        'no_host': const SessionNoHost('Проба к хосту не ответила'),
        'interrupted_other_vpn': const SessionInterrupted(byOtherVpn: true),
        'interrupted': const SessionInterrupted(byOtherVpn: false),
      };
      final cases = fixtureJson('session_states.json')! as List<dynamic>;
      expect(cases, hasLength(expected.length));
      for (final raw in cases) {
        final item = raw as Map<String, dynamic>;
        final name = item['name'] as String;
        expect(
          SessionState.fromJson(item['session'] as Map<String, dynamic>),
          expected[name],
          reason: name,
        );
      }
    });

    test(
      'кадр «ядра ещё нет» читается как не готовый, а подписки в нём нет',
      () {
        final frame = StateFrame.parse(
          '{"ready":false,"state":{"session":{"state":"idle"},"active":null,"engines":[],'
          '"rules":{"sets":[],"mode":null,"user_domains":[],"user_cidrs":[],"apps":{"mode":"all"}},'
          '"subscription":{"host_id":null,"host_name":null,"version":null,"updated_at":null,'
          '"last_refresh_error":null,"revoked":false},"network":null,"banners":[],"admin":false}}',
        );
        expect(frame.ready, isFalse);
        expect(frame.state.hasSubscription, isFalse);
        expect(frame.state.session, const SessionIdle());
        expect(frame.state.rules.apps, const AppRulesAll());
      },
    );

    test('аварийный кадр без состояния — тоже «не готово»', () {
      final frame = StateFrame.parse('{"ready":false}');
      expect(frame.ready, isFalse);
      expect(frame.state, const CoreState());
    });

    test('баннер неизвестного вида пропускается, остальные читаются', () {
      final json = fixtureObject('core_state_full.json');
      final state = json['state'] as Map<String, dynamic>;
      state['banners'] = [
        {'banner': 'from_the_future', 'x': 1},
        {'banner': 'revoked'},
      ];
      final frame = StateFrame.fromJson(json);
      expect(frame.state.banners, [const RevokedBanner()]);
    });

    test(
      'сломанный документ — FormatException с именем поля, а не падение',
      () {
        final json = fixtureObject('core_state_full.json');
        (json['state'] as Map<String, dynamic>)['engines'] = [
          {'id': 'awg'},
        ];
        expect(
          () => StateFrame.fromJson(json),
          throwsA(
            isA<FormatException>().having(
              (e) => e.message,
              'message',
              contains('kind'),
            ),
          ),
        );
        expect(() => StateFrame.parse('{не json'), throwsFormatException);
        expect(() => StateFrame.parse('[]'), throwsFormatException);
      },
    );

    test(
      'неизвестное состояние сессии — ошибка формата, а не молчаливое «Idle»',
      () {
        expect(
          () => SessionState.fromJson({'state': 'teleporting'}),
          throwsA(isA<FormatException>()),
        );
      },
    );
  });

  group('настройки', () {
    test('умолчания эталона: уведомления выключены, kill switch включён', () {
      final defaults =
          fixtureObject('settings.json')['defaults'] as Map<String, dynamic>;
      final settings = Settings.fromJson(defaults);
      expect(settings, const Settings());
      expect(settings.notifySwitches, isFalse);
      expect(settings.killSwitch, isTrue, reason: 'G03, ист. 33');
      expect(settings.autoconnectDefault, Autoconnect.ask);
      expect(settings.dnsMode, DnsMode.rulesOnly);
      expect(settings.toJson(), defaults);
    });

    test(
      'изменённые настройки читаются и пишутся ровно эталонным документом',
      () {
        final changed =
            fixtureObject('settings.json')['changed'] as Map<String, dynamic>;
        final settings = Settings.fromJson(changed);
        expect(settings.autostart, isTrue);
        expect(settings.autoconnectDefault, Autoconnect.never);
        expect(settings.autoconnectNetworks, {
          'wifi:00aa11bb22cc33dd': Autoconnect.connect,
          'gw:99887766aabbccdd': Autoconnect.ask,
        });
        expect(settings.notifySwitches, isTrue);
        expect(settings.killSwitch, isFalse);
        expect(settings.dnsMode, DnsMode.allViaTunnel);
        expect(settings.toJson(), changed);
      },
    );

    test('copyWith меняет одно поле и не трогает остальные', () {
      const base = Settings(
        killSwitch: false,
        autoconnectNetworks: {'n': Autoconnect.never},
      );
      final next = base.copyWith(notifySwitches: true);
      expect(next.notifySwitches, isTrue);
      expect(next.killSwitch, isFalse);
      expect(next.autoconnectNetworks, {'n': Autoconnect.never});
    });
  });

  group('каталог правил', () {
    test('наборы и режимы с исключениями читаются из эталона', () {
      final catalog = RulesCatalog.fromJson(
        fixtureObject('rules_catalog.json'),
      );
      expect(catalog.sets, hasLength(2));
      final google = catalog.setById('google')!;
      expect(google.name, 'Google');
      expect((google.domains, google.ips), (3, 2));
      expect(google.excludeDomains, ['mail.google.com']);
      expect(google.excludeIps, ['142.251.1.0/24']);
      expect(catalog.setById('cursor')!.excludeDomains, isEmpty);

      expect(catalog.modes.map((m) => (m.id, m.name, m.isDefault)), [
        ('work', 'Работа', true),
        ('home', 'Дом', false),
      ]);
      expect(catalog.modeById('work')!.sets, ['google', 'cursor']);
      expect(catalog.modeById('work')!.excludeDomains, ['calendar.google.com']);
      expect(catalog.modeById(null), isNull);
      expect(catalog.setById('нет'), isNull);
    });
  });

  group('ответы на команды', () {
    test('итоги импорта и обновления подписки', () {
      final outcomes = fixtureObject('outcomes.json');
      final imports = outcomes['import'] as Map<String, dynamic>;
      expect(
        ImportOutcome.fromJson(imports['imported'] as Map<String, dynamic>),
        const Imported(),
      );
      expect(
        ImportOutcome.fromJson(
          imports['replace_requested'] as Map<String, dynamic>,
        ),
        const ReplaceRequested(
          currentHost: 'Франкфурт',
          newHost: '198.51.100.20',
        ),
      );
      final refreshes = outcomes['refresh'] as Map<String, dynamic>;
      expect(
        RefreshOutcome.fromJson(refreshes['updated'] as Map<String, dynamic>),
        const Updated(8),
      );
      expect(
        RefreshOutcome.fromJson(
          refreshes['not_modified'] as Map<String, dynamic>,
        ),
        const NotModified(),
      );
    });

    test('отчёты «Проверить связь» читаются по эталону', () {
      final reports =
          (fixtureJson('engine_probe_reports.json')! as List<dynamic>)
              .map((e) => EngineProbeReport.fromJson(e as Map<String, dynamic>))
              .toList();
      expect(reports, [
        const EngineProbeReport(
          engine: 'awg',
          state: EngineStatus.connected,
          rttMs: 41,
          lossPct: 0,
          handshakeAgeS: 9,
        ),
        const EngineProbeReport(
          engine: 'csqtt',
          state: EngineStatus.waitingUser,
          lossPct: 100,
        ),
        const EngineProbeReport(
          engine: 'wg-next',
          state: EngineStatus.degraded,
          rttMs: 2300,
          lossPct: 60,
          handshakeAgeS: 140,
        ),
      ]);
    });
  });

  group('команды ядру', () {
    test('изменения правил пишутся в форме, которую принимает ядро', () {
      final golden =
          fixtureObject('commands.json')['rules_changes']
              as Map<String, dynamic>;
      final built = <String, RulesChange>{
        'toggle_set_off': const RulesChange.toggleSet('google', on: false),
        'toggle_set_on': const RulesChange.toggleSet('cursor', on: true),
        'select_mode': const RulesChange.selectMode('home'),
        'add_user_domain': const RulesChange.addUserDomain('x.y'),
        'remove_user_domain': const RulesChange.removeUserDomain('x.y'),
        'add_user_cidr': const RulesChange.addUserCidr('10.0.0.0/8'),
        'remove_user_cidr': const RulesChange.removeUserCidr('10.0.0.0/8'),
        'set_apps_all': const RulesChange.setApps(AppRulesAll()),
        'set_apps_allowed': const RulesChange.setApps(
          AppRulesAllowed(['org.example.one', 'org.example.two']),
        ),
        'set_apps_excluded': const RulesChange.setApps(
          AppRulesExcluded(['org.bank.app']),
        ),
      };
      expect(
        built.keys.toSet(),
        golden.keys.toSet(),
        reason: 'эталон и таблица разошлись',
      );
      for (final entry in built.entries) {
        expect(entry.value.toJson(), golden[entry.key], reason: entry.key);
      }
    });

    test('режим движка пишется и читается в форме ядра', () {
      final golden =
          fixtureObject('commands.json')['engine_modes']
              as Map<String, dynamic>;
      expect(const EngineMode.auto().toJson(), golden['auto']);
      expect(const EngineMode.pinned('csqtt').toJson(), golden['pinned']);
      expect(
        EngineMode.fromJson(golden['auto'] as Map<String, dynamic>),
        const EngineMode.auto(),
      );
      expect(
        EngineMode.fromJson(golden['pinned'] as Map<String, dynamic>),
        const EngineMode.pinned('csqtt'),
      );
    });

    test('ответы пользователя движкам пишутся в форме ядра', () {
      final golden =
          fixtureObject('commands.json')['user_inputs'] as Map<String, dynamic>;
      expect(
        const UserInput.captchaResult('tok').toJson(),
        golden['captcha_result'],
      );
      expect(
        const UserInput.vkHashes(['a', 'b']).toJson(),
        golden['vk_hashes'],
      );
      expect(const UserInput.vkToken('t').toJson(), golden['vk_token']);
      expect(const UserInput.vkToken('').toJson(), golden['vk_logout']);
    });

    test('значения сравниваются по содержимому, а в JSON не теряются', () {
      expect(
        const RulesChange.addUserDomain('x.y'),
        const RulesChange.addUserDomain('x.y'),
      );
      expect(
        const RulesChange.addUserDomain('x.y'),
        isNot(const RulesChange.addUserDomain('x.z')),
      );
      expect(
        jsonEncode(const RulesChange.setApps(AppRulesAllowed(['a'])).toJson()),
        '{"change":"set_apps","apps":{"mode":"allowed","packages":["a"]}}',
      );
    });
  });
}

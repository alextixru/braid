// Экран «Правила» (ист. 41, 50, 51, 52, 53, 75, 42, 43): общие наборы с
// переключателями и исключениями, пресеты и «Своё», «Мои правила» с
// нормализацией без потери ввода, приложения на Android, перенос файлом.

import 'package:braid/bridge/braid_bridge.dart';
import 'package:braid/models/commands.dart';
import 'package:braid/models/core_state.dart';
import 'package:braid/services/rules_files.dart';
import 'package:braid/services/vpn_channel.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

import 'support/fake_bridge.dart';
import 'support/fakes.dart';
import 'support/pump_app.dart';
import 'support/rig.dart';
import 'support/states.dart';

Rig desktopRig({CoreState? state, FakeRulesFiles? files}) => Rig(
  bridge: FakeBridge(state: state ?? idleState()),
  files: files,
  autoAnswer: false,
);

/// Как в `main.dart`: файловые диалоги есть и на Android — хранилище ядра
/// лежит в `noBackupFilesDir`, системная резервная копия правила не спасёт.
Rig androidRig({CoreState? state, FakeVpn? vpn, FakeRulesFiles? files}) => Rig(
  bridge: FakeBridge(state: state ?? idleState(), caps: androidCaps),
  platform: android,
  vpn: vpn ?? FakeVpn(),
  files: files,
  autoAnswer: false,
);

Future<Rig> openRules(WidgetTester tester, Rig rig) async {
  await pumpApp(tester, rig: rig);
  await openTile(tester, 'tile-rules');
  return rig;
}

Finder ruleField() => find.byKey(const Key('rule-field'));

String ruleFieldText(WidgetTester tester) =>
    tester.widget<TextField>(ruleField()).controller!.text;

void main() {
  group('общие наборы', () {
    testWidgets('наборы с переключателями, счётчиками и исключениями', (
      tester,
    ) async {
      await openRules(tester, desktopRig());

      final google = tester.widget<SwitchListTile>(
        find.byKey(const Key('set-switch-google')),
      );
      final cursor = tester.widget<SwitchListTile>(
        find.byKey(const Key('set-switch-cursor')),
      );
      expect(google.value, isTrue);
      expect(cursor.value, isTrue);
      expect(find.text('Google'), findsOneWidget);
      expect(find.text('Cursor'), findsOneWidget);
      expect(find.textContaining('3 домена · 2 подсети'), findsOneWidget);
      expect(find.textContaining('2 домена'), findsOneWidget);

      // Исключения набора видны сразу: никогда не идут в туннель (ист. 75).
      final exclusions = find.byKey(const Key('set-exclusions-google'));
      expect(exclusions, findsOneWidget);
      final text = tester.widget<Text>(exclusions).data!;
      expect(text, contains('mail.google.com'));
      expect(text, contains('142.251.1.0/24'));
      expect(find.byKey(const Key('set-exclusions-cursor')), findsNothing);
    });

    testWidgets(
      'переключатель набора вызывает set_rules и меняет пресет на «Своё»',
      (tester) async {
        final rig = await openRules(tester, desktopRig());

        await tester.tap(find.byKey(const Key('set-switch-google')));
        await tester.pumpAndSettle();

        expect(rig.bridge.rulesChanges, [
          const RulesChange.toggleSet('google', on: false),
        ]);
        final google = tester.widget<SwitchListTile>(
          find.byKey(const Key('set-switch-google')),
        );
        expect(google.value, isFalse);
        final custom = tester.widget<ChoiceChip>(
          find.byKey(const Key('mode-chip-custom')),
        );
        expect(custom.selected, isTrue);
      },
    );

    testWidgets('набор можно включить обратно', (tester) async {
      final off = RulesView(
        sets: [
          const RuleSetView(
            id: 'google',
            name: 'Google',
            enabled: false,
            domains: 3,
            ips: 2,
          ),
          const RuleSetView(
            id: 'cursor',
            name: 'Cursor',
            enabled: true,
            domains: 2,
            ips: 0,
          ),
        ],
        mode: null,
      );
      final rig = await openRules(
        tester,
        desktopRig(state: idleState(rules: off)),
      );
      await tester.tap(find.byKey(const Key('set-switch-google')));
      await tester.pumpAndSettle();
      expect(rig.bridge.rulesChanges, [
        const RulesChange.toggleSet('google', on: true),
      ]);
    });

    testWidgets(
      'ошибка ядра при переключении показывается, переключатель не врёт',
      (tester) async {
        final bridge = FakeBridge()
          ..rulesError = const BridgeError(
            kind: BridgeErrorKind.failed,
            message: 'Не удалось сохранить правила',
            details: [],
          );
        await openRules(tester, Rig(bridge: bridge, autoAnswer: false));
        await tester.tap(find.byKey(const Key('set-switch-google')));
        await tester.pumpAndSettle();
        expect(find.text('Не удалось сохранить правила'), findsOneWidget);
        final google = tester.widget<SwitchListTile>(
          find.byKey(const Key('set-switch-google')),
        );
        expect(google.value, isTrue);
      },
    );
  });

  group('пресеты', () {
    testWidgets('пресеты подписки и «Своё»; текущий выбран', (tester) async {
      await openRules(tester, desktopRig());
      final work = tester.widget<ChoiceChip>(
        find.byKey(const Key('mode-chip-work')),
      );
      final home = tester.widget<ChoiceChip>(
        find.byKey(const Key('mode-chip-home')),
      );
      final custom = tester.widget<ChoiceChip>(
        find.byKey(const Key('mode-chip-custom')),
      );
      expect(
        (work.selected, home.selected, custom.selected),
        (true, false, false),
      );
    });

    testWidgets(
      'выбор пресета шлёт select_mode, переключатели следуют пресету',
      (tester) async {
        final rig = await openRules(tester, desktopRig());

        await tester.tap(find.byKey(const Key('mode-chip-home')));
        await tester.pumpAndSettle();

        expect(rig.bridge.rulesChanges, [const RulesChange.selectMode('home')]);
        final cursor = tester.widget<SwitchListTile>(
          find.byKey(const Key('set-switch-cursor')),
        );
        expect(cursor.value, isFalse);
        final home = tester.widget<ChoiceChip>(
          find.byKey(const Key('mode-chip-home')),
        );
        expect(home.selected, isTrue);
      },
    );

    testWidgets(
      '«Своё» нельзя выбрать вручную — оно получается переключателями',
      (tester) async {
        final rig = await openRules(tester, desktopRig());
        await tester.tap(
          find.byKey(const Key('mode-chip-custom')),
          warnIfMissed: false,
        );
        await tester.pumpAndSettle();
        expect(rig.bridge.rulesChanges, isEmpty);
      },
    );

    testWidgets('исключения выбранного режима видны', (tester) async {
      await openRules(tester, desktopRig());
      final note = find.byKey(const Key('mode-exclusions'));
      expect(note, findsOneWidget);
      expect(tester.widget<Text>(note).data, contains('calendar.google.com'));
    });
  });

  group('«Мои правила»', () {
    testWidgets(
      'ссылка превращается в домен: подсказка до добавления, после — в списке',
      (tester) async {
        final rig = await openRules(tester, desktopRig());
        await reveal(tester, ruleField());

        await tester.enterText(ruleField(), 'https://x.y/путь');
        await tester.pump();
        expect(find.text('Добавится домен: x.y'), findsOneWidget);

        await tester.tap(find.byKey(const Key('rule-add-button')));
        await tester.pumpAndSettle();

        expect(rig.bridge.rulesChanges, [
          const RulesChange.addUserDomain('x.y'),
        ]);
        expect(find.byKey(const Key('user-domain-x.y')), findsOneWidget);
        expect(ruleFieldText(tester), isEmpty);
      },
    );

    testWidgets('подсеть приводится к виду сети и добавляется', (tester) async {
      final rig = await openRules(tester, desktopRig());
      await reveal(tester, ruleField());

      await tester.enterText(ruleField(), ' 10.1.2.3/8 ');
      await tester.pump();
      expect(find.text('Добавится подсеть: 10.0.0.0/8'), findsOneWidget);

      await tester.tap(find.byKey(const Key('rule-add-button')));
      await tester.pumpAndSettle();
      expect(rig.bridge.rulesChanges, [
        const RulesChange.addUserCidr('10.0.0.0/8'),
      ]);
      expect(find.byKey(const Key('user-cidr-10.0.0.0/8')), findsOneWidget);
    });

    testWidgets(
      'не домен и не подсеть: ошибка на месте, ввод остаётся, ядро не трогали',
      (tester) async {
        final rig = await openRules(tester, desktopRig());
        await reveal(tester, ruleField());

        await tester.enterText(ruleField(), 'привет, мир');
        await tester.tap(find.byKey(const Key('rule-add-button')));
        await tester.pumpAndSettle();

        expect(
          find.text('Не похоже ни на домен, ни на подсеть IPv4'),
          findsOneWidget,
        );
        expect(ruleFieldText(tester), 'привет, мир');
        expect(rig.bridge.rulesChanges, isEmpty);

        // Ошибка пропадает, когда ввод исправлен.
        await tester.enterText(ruleField(), 'example.com');
        await tester.pumpAndSettle(); // сообщение об ошибке гаснет анимацией
        expect(
          find.text('Не похоже ни на домен, ни на подсеть IPv4'),
          findsNothing,
        );
      },
    );

    testWidgets('IP вместо домена подсказывает про подсеть', (tester) async {
      await openRules(tester, desktopRig());
      await reveal(tester, ruleField());
      await tester.enterText(ruleField(), '1.2.3.4');
      await tester.pump();
      // Одиночный IPv4 — это подсеть /32, а не ошибка.
      expect(find.text('Добавится подсеть: 1.2.3.4/32'), findsOneWidget);
    });

    testWidgets('ядро отклонило правило — его текст под полем, ввод остаётся', (
      tester,
    ) async {
      final bridge = FakeBridge()
        ..rulesError = const BridgeError(
          kind: BridgeErrorKind.invalidInput,
          message: 'Домен уже есть в общих наборах',
          details: [],
        );
      await openRules(tester, Rig(bridge: bridge, autoAnswer: false));
      await reveal(tester, ruleField());
      await tester.enterText(ruleField(), 'example.com');
      await tester.tap(find.byKey(const Key('rule-add-button')));
      await tester.pumpAndSettle();
      expect(find.text('Домен уже есть в общих наборах'), findsOneWidget);
      expect(ruleFieldText(tester), 'example.com');
    });

    testWidgets('Enter в поле добавляет правило', (tester) async {
      final rig = await openRules(tester, desktopRig());
      await reveal(tester, ruleField());
      await tester.enterText(ruleField(), 'example.org');
      await tester.testTextInput.receiveAction(TextInputAction.done);
      await tester.pumpAndSettle();
      expect(rig.bridge.rulesChanges, [
        const RulesChange.addUserDomain('example.org'),
      ]);
    });

    testWidgets('свои домены и подсети видны и удаляются', (tester) async {
      final state = idleState(
        rules: const RulesView(
          sets: [],
          mode: 'work',
          userDomains: ['example.org'],
          userCidrs: ['10.20.0.0/16'],
        ),
      );
      final rig = await openRules(tester, desktopRig(state: state));
      await reveal(tester, find.byKey(const Key('user-domain-example.org')));

      expect(find.text('example.org'), findsOneWidget);
      expect(find.text('10.20.0.0/16'), findsOneWidget);

      await tester.tap(find.byKey(const Key('remove-domain-example.org')));
      await tester.pumpAndSettle();
      await tester.tap(find.byKey(const Key('remove-cidr-10.20.0.0/16')));
      await tester.pumpAndSettle();
      expect(rig.bridge.rulesChanges, [
        const RulesChange.removeUserDomain('example.org'),
        const RulesChange.removeUserCidr('10.20.0.0/16'),
      ]);
      expect(find.byKey(const Key('user-domain-example.org')), findsNothing);
    });

    testWidgets('своих правил нет — так и сказано', (tester) async {
      await openRules(tester, desktopRig());
      await reveal(tester, ruleField());
      expect(find.text('Своих правил пока нет'), findsOneWidget);
    });
  });

  group('перенос своих правил файлом (десктоп)', () {
    final withUserRules = idleState(
      rules: const RulesView(
        sets: [],
        mode: 'work',
        userDomains: ['example.org', 'x.y'],
        userCidrs: ['10.20.0.0/16'],
      ),
    );

    testWidgets('экспорт записывает домены и подсети в файл', (tester) async {
      final files = FakeRulesFiles();
      final rig = await openRules(
        tester,
        desktopRig(state: withUserRules, files: files),
      );
      await reveal(tester, find.byKey(const Key('rules-export')));

      await tester.tap(find.byKey(const Key('rules-export')));
      await tester.pumpAndSettle();

      expect(files.saved, hasLength(1));
      expect(files.saved.single.$1, 'braid-rules.json');
      final content = files.saved.single.$2;
      expect(content, contains('"example.org"'));
      expect(content, contains('"x.y"'));
      expect(content, contains('"10.20.0.0/16"'));
      expect(find.textContaining('Сохранено'), findsOneWidget);
      expect(rig.bridge.rulesChanges, isEmpty);
    });

    testWidgets('отмена в диалоге сохранения молчит', (tester) async {
      final files = FakeRulesFiles()..savePath = null;
      await openRules(tester, desktopRig(state: withUserRules, files: files));
      await reveal(tester, find.byKey(const Key('rules-export')));
      await tester.tap(find.byKey(const Key('rules-export')));
      await tester.pumpAndSettle();
      expect(find.textContaining('Сохранено'), findsNothing);
    });

    testWidgets('импорт добавляет правила из файла и сообщает итог', (
      tester,
    ) async {
      final files = FakeRulesFiles()
        ..toOpen =
            '{"format":"braid-rules","version":1,'
            '"domains":["https://news.example.org/a","*.cdn.example.net","нет домена"],'
            '"subnets":["10.1.2.3/8","не подсеть"]}';
      final rig = await openRules(tester, desktopRig(files: files));
      await reveal(tester, find.byKey(const Key('rules-import')));

      await tester.tap(find.byKey(const Key('rules-import')));
      await tester.pumpAndSettle();

      expect(rig.bridge.rulesChanges, [
        const RulesChange.addUserDomain('news.example.org'),
        const RulesChange.addUserDomain('*.cdn.example.net'),
        const RulesChange.addUserCidr('10.0.0.0/8'),
      ]);
      expect(find.textContaining('Добавлено: 3'), findsOneWidget);
      expect(find.textContaining('пропущено: 2'), findsOneWidget);
    });

    testWidgets('не тот файл — понятное сообщение, правила не тронуты', (
      tester,
    ) async {
      final files = FakeRulesFiles()..toOpen = '{"что-то":"другое"}';
      final rig = await openRules(tester, desktopRig(files: files));
      await reveal(tester, find.byKey(const Key('rules-import')));
      await tester.tap(find.byKey(const Key('rules-import')));
      await tester.pumpAndSettle();
      expect(find.text('Это не файл правил Braid'), findsOneWidget);
      expect(rig.bridge.rulesChanges, isEmpty);
    });

    testWidgets('файл от будущей версии не читается вслепую', (tester) async {
      final files = FakeRulesFiles()
        ..toOpen =
            '{"format":"braid-rules","version":2,"domains":[],"subnets":[]}';
      await openRules(tester, desktopRig(files: files));
      await reveal(tester, find.byKey(const Key('rules-import')));
      await tester.tap(find.byKey(const Key('rules-import')));
      await tester.pumpAndSettle();
      expect(
        find.text('Файл правил другой версии — обнови Braid'),
        findsOneWidget,
      );
    });

    testWidgets(
      'на Android экспорт и импорт тоже есть — резервная копия правила не хранит',
      (tester) async {
        final files = FakeRulesFiles();
        final rig = await openRules(
          tester,
          androidRig(state: withUserRules, files: files),
        );
        await reveal(tester, find.byKey(const Key('rules-export')));
        expect(find.byKey(const Key('rules-import')), findsOneWidget);
        await tester.tap(find.byKey(const Key('rules-export')));
        await tester.pumpAndSettle();
        expect(files.saved, hasLength(1));
        expect(files.saved.single.$2, contains('"example.org"'));
        expect(rig.bridge.rulesChanges, isEmpty);
      },
    );

    test('файловые диалоги приложения доступны и на Android, не только на десктопе', () {
      expect(rulesFilesFor(macOS).available, isTrue);
      expect(
        rulesFilesFor(android).available,
        isTrue,
        reason:
            'хранилище ядра на Android — noBackupFilesDir, резервная копия '
            'правила не хранит; переносить их можно только файлом',
      );
    });
  });

  group('приложения (Android)', () {
    Finder appsMode() => find.byKey(const Key('apps-mode'));

    testWidgets('на десктопе блока приложений нет', (tester) async {
      await openRules(tester, desktopRig());
      expect(appsMode(), findsNothing);
    });

    testWidgets('режим «по спискам» и список исключённых приложений', (
      tester,
    ) async {
      final state = idleState(
        rules: const RulesView(
          sets: [],
          mode: 'work',
          apps: AppRulesExcluded(['org.bank.app']),
        ),
      );
      await openRules(tester, androidRig(state: state));
      await reveal(tester, appsMode());

      SegmentedButton<String> mode() => tester.widget(appsMode());
      expect(mode().selected, {'lists'});
      expect(find.byKey(const Key('app-org.bank.app')), findsOneWidget);
      expect(find.textContaining('всегда напрямую'), findsWidgets);
    });

    testWidgets(
      'смена режима при подключении предупреждает о разрыве; отказ ничего не меняет',
      (tester) async {
        final rig = await openRules(
          tester,
          androidRig(state: connectedState()),
        );
        await reveal(tester, appsMode());

        await tester.tap(
          find.descendant(
            of: appsMode(),
            matching: find.text('По приложениям'),
          ),
        );
        await tester.pumpAndSettle();
        expect(find.text('Сменить режим?'), findsOneWidget);
        expect(find.textContaining('разорвёт'), findsOneWidget);

        await tester.tap(find.text('Отмена'));
        await tester.pumpAndSettle();
        expect(rig.bridge.rulesChanges, isEmpty);
      },
    );

    testWidgets('согласие на разрыв переключает режим на «по приложениям»', (
      tester,
    ) async {
      final rig = await openRules(tester, androidRig(state: connectedState()));
      await reveal(tester, appsMode());
      await tester.tap(
        find.descendant(of: appsMode(), matching: find.text('По приложениям')),
      );
      await tester.pumpAndSettle();
      await tester.tap(find.byKey(const Key('apps-mode-confirm')));
      await tester.pumpAndSettle();

      expect(rig.bridge.rulesChanges, [
        const RulesChange.setApps(AppRulesAllowed([])),
      ]);
      SegmentedButton<String> mode() => tester.widget(appsMode());
      expect(mode().selected, {'apps'});
      // Пустой выбор ядро считает обычным режимом по спискам — так и сказано.
      expect(
        find.textContaining(
          'ни одно приложение не выбрано — всё работает по спискам',
        ),
        findsOneWidget,
      );
    });

    testWidgets('не подключено — режим меняется без предупреждения', (
      tester,
    ) async {
      final rig = await openRules(tester, androidRig());
      await reveal(tester, appsMode());
      await tester.tap(
        find.descendant(of: appsMode(), matching: find.text('По приложениям')),
      );
      await tester.pumpAndSettle();
      expect(find.text('Сменить режим?'), findsNothing);
      expect(rig.bridge.rulesChanges, [
        const RulesChange.setApps(AppRulesAllowed([])),
      ]);
    });

    testWidgets('приложение добавляется из списка установленных', (
      tester,
    ) async {
      final vpn = FakeVpn(
        apps: const [
          InstalledApp(package: 'org.bank.app', label: 'Банк'),
          InstalledApp(package: 'org.chat.app', label: 'Чат'),
        ],
      );
      final rig = await openRules(tester, androidRig(vpn: vpn));
      await reveal(tester, find.byKey(const Key('app-add')));

      await tester.tap(find.byKey(const Key('app-add')));
      await tester.pumpAndSettle();
      expect(find.text('Банк'), findsOneWidget);
      await tester.tap(find.text('Банк'));
      await tester.pumpAndSettle();

      expect(rig.bridge.rulesChanges, [
        const RulesChange.setApps(AppRulesExcluded(['org.bank.app'])),
      ]);
      expect(find.byKey(const Key('app-org.bank.app')), findsOneWidget);
    });

    testWidgets(
      'список установленных недоступен — имя пакета вводится вручную с проверкой',
      (tester) async {
        final rig = await openRules(tester, androidRig());
        await reveal(tester, find.byKey(const Key('app-add')));

        await tester.tap(find.byKey(const Key('app-add')));
        await tester.pumpAndSettle();
        await tester.enterText(
          find.byKey(const Key('app-package-field')),
          'не пакет',
        );
        await tester.tap(find.byKey(const Key('app-package-add')));
        await tester.pumpAndSettle();
        expect(find.textContaining('Имя пакета'), findsWidgets);
        expect(rig.bridge.rulesChanges, isEmpty);

        await tester.enterText(
          find.byKey(const Key('app-package-field')),
          'org.bank.app',
        );
        await tester.tap(find.byKey(const Key('app-package-add')));
        await tester.pumpAndSettle();
        expect(rig.bridge.rulesChanges, [
          const RulesChange.setApps(AppRulesExcluded(['org.bank.app'])),
        ]);
      },
    );

    testWidgets('удаление последнего приложения возвращает «все по спискам»', (
      tester,
    ) async {
      final state = idleState(
        rules: const RulesView(
          sets: [],
          mode: 'work',
          apps: AppRulesExcluded(['org.bank.app']),
        ),
      );
      final rig = await openRules(tester, androidRig(state: state));
      await reveal(tester, find.byKey(const Key('app-remove-org.bank.app')));
      await tester.tap(find.byKey(const Key('app-remove-org.bank.app')));
      await tester.pumpAndSettle();
      expect(rig.bridge.rulesChanges, [
        const RulesChange.setApps(AppRulesAll()),
      ]);
    });
  });
}

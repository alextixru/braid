// Системные швы приложения: каналы к родному слою (сервис Android, deeplink
// десктопа), буфер обмена, мелкие настройки интерфейса, открытие ссылок.
// Родной код подменён обработчиком канала — проверяется контракт имён и форм.

import 'package:braid/services/clipboard_access.dart';
import 'package:braid/services/deep_links.dart';
import 'package:braid/services/ui_prefs.dart';
import 'package:braid/services/url_opener.dart';
import 'package:braid/services/vpn_channel.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:shared_preferences/shared_preferences.dart';

void main() {
  TestWidgetsFlutterBinding.ensureInitialized();
  final messenger =
      TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger;

  void handle(
    String channel,
    Future<Object?>? Function(MethodCall call) handler,
  ) {
    messenger.setMockMethodCallHandler(MethodChannel(channel), handler);
    addTearDown(
      () => messenger.setMockMethodCallHandler(MethodChannel(channel), null),
    );
  }

  group('канал сервиса Android org.braid/vpn', () {
    test(
      'имена и ответы методов совпадают с контрактом тикетов 09 ↔ 10',
      () async {
        final calls = <String>[];
        handle(MethodChannelVpn.name, (call) async {
          calls.add(call.method);
          return switch (call.method) {
            'prepare' => true,
            'startService' => true,
            'isServiceRunning' => true,
            'shareDomainPending' => 'site.com',
            'linkPending' => 'braid://pending',
            _ => null,
          };
        });
        const vpn = MethodChannelVpn();
        expect(vpn.available, isTrue);
        expect(await vpn.prepare(), isTrue);
        expect(await vpn.startService(), isTrue);
        await vpn.stopService();
        expect(await vpn.isServiceRunning(), isTrue);
        expect(await vpn.takeSharedDomain(), 'site.com');
        expect(await vpn.takePendingLink(), 'braid://pending');
        expect(calls, [
          'prepare',
          'startService',
          'stopService',
          'isServiceRunning',
          'shareDomainPending',
          'linkPending',
        ]);
      },
    );

    test(
      'ядро без подключения: метод initCore, а где его нет — запуск сервиса',
      () async {
        final calls = <String>[];
        handle(MethodChannelVpn.name, (call) async {
          calls.add(call.method);
          return call.method == 'initCore' ? true : null;
        });
        expect(await const MethodChannelVpn().ensureCore(), isTrue);
        expect(calls, [
          'initCore',
        ], reason: 'сервис не стартует и не подключается');

        // Родной слой без initCore: единственный путь поднять ядро — запуск сервиса.
        calls.clear();
        handle(MethodChannelVpn.name, (call) async {
          calls.add(call.method);
          if (call.method == 'initCore') throw MissingPluginException();
          return true;
        });
        expect(await const MethodChannelVpn().ensureCore(), isTrue);
        expect(calls, ['initCore', 'startService']);

        calls.clear();
        handle(MethodChannelVpn.name, (call) async {
          calls.add(call.method);
          if (call.method == 'initCore') {
            throw PlatformException(code: 'notImplemented');
          }
          return true;
        });
        expect(await const MethodChannelVpn().ensureCore(), isTrue);
        expect(calls, ['initCore', 'startService']);
      },
    );

    test('отказ в согласии VPN — false, а не ошибка', () async {
      handle(
        MethodChannelVpn.name,
        (call) async => call.method == 'prepare' ? false : null,
      );
      expect(await const MethodChannelVpn().prepare(), isFalse);
    });

    test(
      'список приложений: пакеты и подписи; без подписи — имя пакета',
      () async {
        handle(MethodChannelVpn.name, (call) async {
          if (call.method != 'installedApps') return null;
          return [
            {'package': 'org.bank.app', 'label': 'Банк'},
            {'package': 'org.plain.app'},
            {'label': 'без пакета'},
          ];
        });
        final apps = await const MethodChannelVpn().installedApps();
        expect(apps!.map((a) => (a.package, a.label)), [
          ('org.bank.app', 'Банк'),
          ('org.plain.app', 'org.plain.app'),
        ]);
      },
    );

    test('родной слой не знает installedApps или linkPending — это «нет», а не сбой', () async {
      // Обработчика нет совсем: канал отвечает MissingPluginException.
      const vpn = MethodChannelVpn();
      expect(await vpn.installedApps(), isNull);
      expect(await vpn.takePendingLink(), isNull);
      handle(MethodChannelVpn.name, (call) async {
        throw PlatformException(code: 'notImplemented');
      });
      expect(await vpn.installedApps(), isNull);
    });

    test('чужая ошибка канала не глотается', () async {
      handle(MethodChannelVpn.name, (call) async {
        throw PlatformException(code: 'security', message: 'нет права');
      });
      await expectLater(
        const MethodChannelVpn().installedApps(),
        throwsA(isA<PlatformException>()),
      );
    });

    test('на десктопе сервиса нет, и ничего не падает', () async {
      const vpn = NoVpnChannel();
      expect(vpn.available, isFalse);
      expect(await vpn.prepare(), isTrue);
      expect(await vpn.takeSharedDomain(), isNull);
      expect(await vpn.installedApps(), isNull);
    });
  });

  group('deeplink десктопа org.braid/links', () {
    test('ссылка запуска берётся у родного слоя', () async {
      handle(
        ChannelDeepLinks.name,
        (call) async => call.method == 'initialLink' ? 'braid://launch' : null,
      );
      expect(await ChannelDeepLinks().initialLink(), 'braid://launch');
    });

    test('нет родного слоя ссылок — запуск обычный', () async {
      expect(await ChannelDeepLinks().initialLink(), isNull);
    });

    test('ссылки, пришедшие при работе, попадают в поток', () async {
      final links = ChannelDeepLinks();
      final received = <String>[];
      final sub = links.links.listen(received.add);
      addTearDown(sub.cancel);
      await messenger.handlePlatformMessage(
        ChannelDeepLinks.name,
        const StandardMethodCodec().encodeMethodCall(
          const MethodCall('onLink', 'braid://live'),
        ),
        (_) {},
      );
      ByteData? reply;
      await messenger.handlePlatformMessage(
        ChannelDeepLinks.name,
        const StandardMethodCodec().encodeMethodCall(
          const MethodCall('onLink', 42),
        ),
        (data) => reply = data,
      );
      // Не-строка — не ссылка, но и не сбой родного слоя: ответ без ошибки.
      expect(
        () => const StandardMethodCodec().decodeEnvelope(reply!),
        returnsNormally,
      );
      await pumpEventQueue();
      expect(received, [
        'braid://live',
      ], reason: 'не-строка не считается ссылкой');
    });
  });

  // Платформенный канал Flutter говорит JSON, а не стандартным кодеком.
  void handlePlatform(Future<Object?>? Function(MethodCall call) handler) {
    messenger.setMockMethodCallHandler(SystemChannels.platform, handler);
    addTearDown(
      () => messenger.setMockMethodCallHandler(SystemChannels.platform, null),
    );
  }

  group('буфер обмена', () {
    test('читает и пишет обычный текст', () async {
      var stored = 'braid://x';
      handlePlatform((call) async {
        if (call.method == 'Clipboard.getData') return {'text': stored};
        if (call.method == 'Clipboard.setData') {
          stored = (call.arguments as Map)['text'] as String;
        }
        return null;
      });
      const clipboard = SystemClipboard();
      expect(await clipboard.readText(), 'braid://x');
      await clipboard.writeText('braid://y');
      expect(await clipboard.readText(), 'braid://y');
    });

    test('пустой буфер — null', () async {
      handlePlatform((call) async => null);
      expect(await const SystemClipboard().readText(), isNull);
    });
  });

  group('мелочи интерфейса', () {
    test(
      'отказ от sudoers и закрытое обновление переживают перезапуск',
      () async {
        SharedPreferences.setMockInitialValues({});
        const prefs = SharedUiPrefs();
        expect(await prefs.sudoersOfferDeclined(), isFalse);
        expect(await prefs.dismissedUpdate(), isNull);

        await prefs.setSudoersOfferDeclined(true);
        await prefs.setDismissedUpdate('0.2.0');
        expect(await const SharedUiPrefs().sudoersOfferDeclined(), isTrue);
        expect(await const SharedUiPrefs().dismissedUpdate(), '0.2.0');
      },
    );
  });

  group('открытие ссылок', () {
    test('только https: остальное не уходит системе', () async {
      const opener = SystemUrlOpener();
      for (final url in [
        'http://example.org/Braid.zip',
        'javascript:alert(1)',
        'file:///etc/passwd',
        'braid://x',
        'не ссылка',
        'https://',
        '',
      ]) {
        expect(await opener.open(url), isFalse, reason: url);
      }
    });
  });
}

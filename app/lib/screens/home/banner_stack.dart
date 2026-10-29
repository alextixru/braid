import 'package:braid/controller/braid_scope.dart';
import 'package:braid/controller/connect_models.dart';
import 'package:braid/models/core_state.dart';
import 'package:braid/screens/subscription/subscription_screen.dart';
import 'package:braid/util/format.dart';
import 'package:braid/widgets/banner_card.dart';
import 'package:flutter/material.dart';

/// Плашки главного экрана: то, что требует внимания, но не мешает пользоваться
/// (обновление, хеши VK, давняя подписка, отозванный доступ, новая сеть,
/// правило sudoers, нечитаемый кадр ядра).
class BannerStack extends StatelessWidget {
  const BannerStack({super.key});

  @override
  Widget build(BuildContext context) {
    final controller = BraidScope.of(context);
    final extensions = BraidScope.extensionsOf(context);
    void openEngines() =>
        Navigator.of(context)
            .push(MaterialPageRoute<void>(builder: extensions.engines!));
    void openSubscription() => Navigator.of(
      context,
    ).push(MaterialPageRoute<void>(builder: (_) => const SubscriptionScreen()));

    final cards = <Widget>[
      if (controller.protocolError != null)
        BannerCard(
          key: const Key('banner-protocol'),
          icon: Icons.error_outline,
          title: controller.protocolError!,
          isProblem: true,
        ),
      if (controller.ruleNeedsUpdate)
        BannerCard(
          key: const Key('banner-sudoers'),
          icon: Icons.admin_panel_settings_outlined,
          title: 'Обновить правило для нового помощника',
          subtitle:
              'Приложение обновилось: пока правило старое, при подключении '
              'спрашивается пароль администратора.',
          actions: [
            TextButton(
              key: const Key('banner-sudoers-update'),
              onPressed: controller.installSudoersRule,
              child: const Text('Обновить'),
            ),
          ],
        ),
      for (final banner in controller.banners)
        switch (banner) {
          UpdateAvailableBanner(:final latest, :final url) => BannerCard(
            key: const Key('banner-update'),
            icon: Icons.system_update_alt,
            title: 'Доступна версия $latest',
            subtitle: 'Обновление не срочное: всё работает и на этой версии.',
            actions: [
              if (url != null)
                TextButton(
                  key: const Key('banner-update-open'),
                  onPressed: () => controller.openUrl(url),
                  child: const Text('Скачать'),
                ),
            ],
            onClose: () => controller.dismissUpdate(latest),
            closeKey: const Key('banner-update-close'),
          ),
          NeedsHashesBanner() => BannerCard(
            key: const Key('banner-hashes'),
            icon: Icons.link_off,
            title: 'CSQTT: нужны хеши звонков VK',
            subtitle:
                'Без них CSQTT не подключится; связь держит другой движок.',
            actions: [
              if (extensions.engines != null)
                TextButton(
                  key: const Key('banner-hashes-open'),
                  onPressed: openEngines,
                  child: const Text('Движки'),
                ),
            ],
          ),
          NeedsVkLoginBanner() => BannerCard(
            key: const Key('banner-vk'),
            icon: Icons.login,
            title: 'CSQTT: нужен вход через VK',
            subtitle: 'Войди через VK или введи хеши звонков — связь пока держит другой движок.',
            actions: [
              if (extensions.engines != null)
                TextButton(
                  key: const Key('banner-vk-open'),
                  onPressed: openEngines,
                  child: const Text('Движки'),
                ),
            ],
          ),
          SubscriptionStaleBanner(:final since) => BannerCard(
            key: const Key('banner-stale'),
            icon: Icons.sync_problem,
            title:
                'Подписка не обновлялась с '
                '${formatWhen(since.toLocal(), now: controller.now())}',
            subtitle: 'Работает последняя сохранённая копия.',
            actions: [
              TextButton(
                key: const Key('banner-stale-refresh'),
                onPressed: controller.refreshing
                    ? null
                    : controller.refreshSubscription,
                child: const Text('Обновить'),
              ),
            ],
          ),
          RevokedBanner() => BannerCard(
            key: const Key('banner-revoked'),
            icon: Icons.block,
            title: 'Доступ отозван',
            subtitle: 'Хост закрыл твой доступ. Запроси у администратора новую ссылку.',
            isProblem: true,
            actions: [
              TextButton(
                key: const Key('banner-revoked-new-link'),
                onPressed: openSubscription,
                child: const Text('Новая ссылка'),
              ),
            ],
          ),
          AutoconnectAskBanner(:final network) => BannerCard(
            key: const Key('banner-autoconnect'),
            icon: Icons.wifi,
            title: 'Новая сеть: ${networkLabel(network)}',
            subtitle: 'Подключаться к VPN в этой сети?',
            actions: [
              for (final (key, label, answer) in [
                ('once', 'Сейчас', AutoconnectAnswer.once),
                ('always', 'Всегда', AutoconnectAnswer.always),
                ('never', 'Никогда', AutoconnectAnswer.never),
              ])
                TextButton(
                  key: Key('banner-autoconnect-$key'),
                  onPressed: () =>
                      controller.answerAutoconnect(network, answer),
                  child: Text(label),
                ),
            ],
          ),
          // Конфликт с другим VPN показан в статусе кнопки.
          OtherVpnBanner() => const SizedBox.shrink(),
        },
    ];
    if (cards.isEmpty) return const SizedBox.shrink();
    return Column(
      children: [
        for (final card in cards)
          Padding(padding: const EdgeInsets.only(bottom: 8), child: card),
      ],
    );
  }
}

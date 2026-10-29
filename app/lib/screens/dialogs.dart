/// Диалоги подтверждений: замена подписки, добавление домена, правило sudoers.
/// Каждый возвращает ответ пользователя; закрытие мимо кнопок — «нет».
library;

import 'package:braid/app.dart' show appName;
import 'package:braid/controller/ui_requests.dart';
import 'package:flutter/material.dart';

Future<bool> showConfirmReplace(
  BuildContext context,
  ConfirmReplace request,
) async {
  final answer = await showDialog<bool>(
    context: context,
    barrierDismissible: false,
    builder: (context) => AlertDialog(
      key: const Key('confirm-replace'),
      title: const Text('Заменить подписку?'),
      content: Text(
        'Сейчас подключена подписка «${request.currentHost}». '
        'Новая ссылка ведёт на «${request.newHost}».\n\n'
        'После замены будут хост и движки новой подписки. '
        'Твои домены и подсети останутся на месте.',
      ),
      actions: [
        TextButton(
          onPressed: () => Navigator.pop(context, false),
          child: const Text('Отмена'),
        ),
        FilledButton(
          key: const Key('confirm-replace-yes'),
          onPressed: () => Navigator.pop(context, true),
          child: const Text('Заменить'),
        ),
      ],
    ),
  );
  return answer ?? false;
}

Future<bool> showConfirmDomain(
  BuildContext context,
  ConfirmDomain request,
) async {
  final answer = await showDialog<bool>(
    context: context,
    builder: (context) => AlertDialog(
      key: const Key('confirm-domain'),
      title: Text('Добавить ${request.domain}?'),
      content: const Text(
        'Домен и его поддомены пойдут через VPN сразу, без переподключения.',
      ),
      actions: [
        TextButton(
          onPressed: () => Navigator.pop(context, false),
          child: const Text('Отмена'),
        ),
        FilledButton(
          key: const Key('confirm-domain-yes'),
          onPressed: () => Navigator.pop(context, true),
          child: const Text('Добавить'),
        ),
      ],
    ),
  );
  return answer ?? false;
}

/// Правило sudoers: пользователь видит, что именно будет установлено, и
/// соглашается на ту же команду.
Future<SudoersAnswer> showSudoersOffer(
  BuildContext context,
  OfferSudoers request,
) async {
  final plan = request.plan;
  final theme = Theme.of(context);
  final mono = theme.textTheme.bodySmall?.copyWith(fontFamily: 'monospace');
  final answer = await showDialog<SudoersAnswer>(
    context: context,
    barrierDismissible: false,
    builder: (context) => AlertDialog(
      key: const Key('sudoers-offer'),
      title: Text(
        request.update
            ? 'Обновить правило для нового помощника?'
            : 'Не спрашивать пароль в дальнейшем?',
      ),
      content: SizedBox(
        width: 480,
        child: SingleChildScrollView(
          child: Column(
            mainAxisSize: MainAxisSize.min,
            crossAxisAlignment: CrossAxisAlignment.start,
            children: [
              Text(
                request.update
                    ? 'Приложение обновилось, а правило sudoers привязано к прошлой '
                          'сборке помощника. Пока оно не обновлено, при каждом '
                          'подключении спрашивается пароль администратора.'
                    : 'Для подключения $appName запускает помощника с правами '
                          'администратора. Чтобы не вводить пароль каждый раз, можно один раз '
                          'добавить правило sudoers — только для этого помощника.',
              ),
              const SizedBox(height: 16),
              Text(
                'Команда установки (от администратора)',
                style: theme.textTheme.labelLarge,
              ),
              const SizedBox(height: 4),
              SelectableText(
                plan.command,
                key: const Key('sudoers-command'),
                style: mono,
              ),
              const SizedBox(height: 12),
              Text('Правило в ${plan.path}', style: theme.textTheme.labelLarge),
              const SizedBox(height: 4),
              SelectableText(
                plan.rule,
                key: const Key('sudoers-rule'),
                style: mono,
              ),
            ],
          ),
        ),
      ),
      actions: [
        TextButton(
          key: const Key('sudoers-later'),
          onPressed: () => Navigator.pop(context, SudoersAnswer.later),
          child: const Text('Не сейчас'),
        ),
        FilledButton(
          key: const Key('sudoers-install'),
          onPressed: () => Navigator.pop(context, SudoersAnswer.install),
          child: const Text('Установить правило'),
        ),
      ],
    ),
  );
  return answer ?? SudoersAnswer.later;
}

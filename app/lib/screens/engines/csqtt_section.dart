/// CSQTT на экране «Движки» (ист. 14–17, 78): список хешей звонков с проверкой,
/// вход через VK, подсказка «Нужны новые хеши».
library;

import 'dart:async';

import 'package:braid/app.dart' show appName;
import 'package:braid/controller/braid_scope.dart';
import 'package:braid/models/core_state.dart';
import 'package:braid/screens/engines/engine_services.dart';
import 'package:braid/screens/engines/web_flow_dialog.dart';
import 'package:braid/services/vk/call_hash.dart';
import 'package:braid/services/vk/csqtt_credentials.dart';
import 'package:braid/services/vk/vk_oauth.dart';
import 'package:braid/widgets/banner_card.dart';
import 'package:braid/widgets/section_header.dart';
import 'package:flutter/material.dart';

class CsqttSection extends StatefulWidget {
  const CsqttSection({super.key});

  @override
  State<CsqttSection> createState() => _CsqttSectionState();
}

class _CsqttSectionState extends State<CsqttSection> {
  final _input = TextEditingController();

  /// Ошибка под полем ввода; ввод при ошибке остаётся как есть.
  String? _inputError;

  /// Итог последнего входа или выхода: отказ или предупреждение.
  String? _loginNote;
  bool _loggingIn = false;

  @override
  void dispose() {
    _input.dispose();
    super.dispose();
  }

  Future<void> _add(CsqttCredentials credentials) async {
    final text = _input.text;
    final error = text.trim().isEmpty
        ? 'Вставь ссылку на звонок VK или хеш'
        : await credentials.addHash(text);
    if (!mounted) return;
    setState(() {
      _inputError = error;
      if (error == null) _input.clear();
    });
  }

  Future<void> _login(CsqttCredentials credentials) async {
    if (_loggingIn) return;
    final navigatorKey = BraidScope.navigatorKeyOf(context);
    setState(() {
      _loggingIn = true;
      _loginNote = null;
    });
    try {
      final flow = await credentials.beginLogin();
      final result = await presentWebFlow<VkLoginResult>(
        navigatorKey: navigatorKey,
        flow: flow,
        title: 'Вход через VK',
        hint:
            'Войди в VK в открывшемся окне. $appName получит токен сам и '
            'закроет его.',
        pageHint: 'Войди в VK — $appName получит токен сам.',
        dialogKey: const Key('vk-login-dialog'),
      );
      final note = await credentials.completeLogin(result);
      if (mounted) setState(() => _loginNote = note);
    } finally {
      if (mounted) setState(() => _loggingIn = false);
    }
  }

  Future<void> _logout(CsqttCredentials credentials) async {
    final note = await credentials.logout();
    if (mounted) setState(() => _loginNote = note);
  }

  @override
  Widget build(BuildContext context) {
    // Кадры ядра (движок попросил хеши, капчу) перестраивают раздел.
    BraidScope.of(context);
    final credentials = EngineServices.of(BraidScope.read(context)).csqtt;
    return ListenableBuilder(
      listenable: credentials,
      builder: (context, _) => _content(context, credentials),
    );
  }

  Widget _content(BuildContext context, CsqttCredentials credentials) {
    final theme = Theme.of(context);
    final engine = credentials.engine;
    final rejected = credentials.tokenRejected;
    final useToken = credentials.loggedIn && !rejected;
    final needHashes = engine?.userAction is NeedHashesAction;
    final statusText = rejected
        ? 'VK: токен не принят — войди заново'
        : credentials.loggedIn
        ? 'VK: вошли'
        : 'VK: не вошли';
    return Column(
      key: const Key('csqtt-section'),
      crossAxisAlignment: CrossAxisAlignment.start,
      children: [
        const SectionHeader(
          'CSQTT · звонки VK',
          subtitle: 'Либо свои хеши звонков, либо вход через VK.',
        ),
        if (needHashes) ...[
          BannerCard(
            key: const Key('csqtt-hint'),
            icon: Icons.link_off,
            title: 'Нужны новые хеши',
            subtitle:
                'Сохранённые звонки закончились или протухли. Создай в VK '
                'новый звонок, скопируй ссылку-приглашение вида '
                'https://vk.com/call/join/… и вставь её ниже. Или войди через '
                'VK — тогда хеши не нужны.',
          ),
          const SizedBox(height: 8),
        ],
        if (engine?.userAction is CaptchaAction) ...[
          Text(
            'VK просит капчу — окно для её решения открыто.',
            key: const Key('csqtt-captcha'),
            style: theme.textTheme.bodyMedium,
          ),
          const SizedBox(height: 8),
        ],
        Card(
          child: Column(
            crossAxisAlignment: CrossAxisAlignment.start,
            children: [
              ListTile(
                key: const Key('vk-status'),
                leading: Icon(
                  useToken
                      ? Icons.verified_user_outlined
                      : Icons.person_outline,
                ),
                title: Text(statusText),
                subtitle: _loginNote == null
                    ? null
                    : Text(
                        _loginNote!,
                        key: const Key('vk-note'),
                        style: TextStyle(color: theme.colorScheme.error),
                      ),
                trailing: useToken
                    ? TextButton(
                        key: const Key('vk-logout'),
                        onPressed: () => _logout(credentials),
                        child: const Text('Выйти'),
                      )
                    : FilledButton.tonal(
                        key: const Key('vk-login'),
                        onPressed: _loggingIn
                            ? null
                            : () => _login(credentials),
                        child: const Text('Войти через VK'),
                      ),
              ),
              const Divider(height: 1),
              if (useToken)
                _tokenNote(context, credentials)
              else
                ..._manualHashes(context, credentials, engine),
            ],
          ),
        ),
      ],
    );
  }

  /// После входа звонки создаются сами; сохранённые хеши ждут выхода.
  Widget _tokenNote(BuildContext context, CsqttCredentials credentials) {
    final theme = Theme.of(context);
    final saved = credentials.hashes.length;
    return Padding(
      padding: const EdgeInsets.all(16),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          const Text('Хеши вводить не нужно — звонки создаются сами'),
          if (saved > 0)
            Text(
              'Сохранённых хешей: $saved — они вернутся после выхода.',
              style: theme.textTheme.bodySmall?.copyWith(
                color: theme.colorScheme.onSurfaceVariant,
              ),
            ),
          if (!credentials.tokenPersisted)
            Text(
              'Вход не сохранён в хранилище устройства и пропадёт при выходе '
              'из приложения.',
              style: theme.textTheme.bodySmall?.copyWith(
                color: theme.colorScheme.error,
              ),
            ),
        ],
      ),
    );
  }

  List<Widget> _manualHashes(
    BuildContext context,
    CsqttCredentials credentials,
    EngineView? engine,
  ) {
    final theme = Theme.of(context);
    final hashes = credentials.hashes;
    return [
      Padding(
        padding: const EdgeInsets.fromLTRB(16, 16, 16, 8),
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: [
            TextField(
              key: const Key('hash-input'),
              controller: _input,
              // Хеш звонка — секрет: клавиатура не запоминает и не исправляет.
              autocorrect: false,
              enableSuggestions: false,
              keyboardType: TextInputType.url,
              decoration: InputDecoration(
                labelText: 'Ссылка на звонок VK или хеш',
                errorText: _inputError,
              ),
              onChanged: (_) {
                if (_inputError != null) setState(() => _inputError = null);
              },
              onSubmitted: (_) => _add(credentials),
            ),
            const SizedBox(height: 8),
            Align(
              alignment: Alignment.centerRight,
              child: FilledButton.tonal(
                key: const Key('hash-add'),
                onPressed: () => _add(credentials),
                child: const Text('Добавить'),
              ),
            ),
          ],
        ),
      ),
      for (var i = 0; i < hashes.length; i++)
        ListTile(
          key: Key('hash-$i'),
          dense: true,
          leading: const Icon(Icons.link),
          title: Text(
            maskHash(hashes[i]),
            style: theme.textTheme.bodyMedium?.copyWith(
              fontFamily: 'monospace',
            ),
          ),
          subtitle: Text(_hashStatus(engine, credentials)),
          trailing: IconButton(
            key: Key('hash-remove-$i'),
            tooltip: 'Удалить',
            icon: const Icon(Icons.delete_outline),
            onPressed: () => unawaited(credentials.removeHash(hashes[i])),
          ),
        ),
      if (hashes.isEmpty)
        Padding(
          padding: const EdgeInsets.fromLTRB(16, 0, 16, 16),
          child: Text(
            'Хешей пока нет. Вставь ссылку на звонок VK или войди через VK.',
            style: theme.textTheme.bodySmall?.copyWith(
              color: theme.colorScheme.onSurfaceVariant,
            ),
          ),
        ),
    ];
  }

  /// Ядро не говорит, какой из звонков отказал, — только что отказали все
  /// (`NeedsHashes`). Поэтому статус общий для списка.
  String _hashStatus(EngineView? engine, CsqttCredentials credentials) {
    if (engine?.userAction is NeedHashesAction) return 'Звонок недоступен';
    final running =
        engine != null &&
        (engine.state == EngineStatus.connected ||
            engine.state == EngineStatus.degraded ||
            engine.state == EngineStatus.starting);
    return running ? 'Используется' : 'Формат проверен';
  }
}

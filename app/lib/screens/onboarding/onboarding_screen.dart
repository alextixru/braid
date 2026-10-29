import 'package:braid/app.dart' show appName;
import 'package:braid/controller/braid_controller.dart';
import 'package:braid/controller/braid_scope.dart';
import 'package:braid/controller/connect_models.dart';
import 'package:flutter/material.dart';

/// Первый запуск: подписки нет, и это единственный экран (ист. 4). Поле для
/// ссылки, вставка из буфера, сканер QR (Android). Ошибка показывается под
/// полем, введённое не стирается.
class OnboardingScreen extends StatefulWidget {
  const OnboardingScreen({super.key});

  @override
  State<OnboardingScreen> createState() => _OnboardingScreenState();
}

class _OnboardingScreenState extends State<OnboardingScreen> {
  final _field = TextEditingController();
  String? _error;
  bool _busy = false;

  @override
  void dispose() {
    _field.dispose();
    super.dispose();
  }

  BraidController get _controller => BraidScope.read(context);

  Future<void> _paste() async {
    final text = (await _controller.clipboard.readText())?.trim() ?? '';
    if (!mounted) return;
    if (text.isEmpty) {
      ScaffoldMessenger.of(context)
        ..hideCurrentSnackBar()
        ..showSnackBar(const SnackBar(content: Text('В буфере пусто')));
      return;
    }
    setState(() {
      _field.value = TextEditingValue(
        text: text,
        selection: TextSelection.collapsed(offset: text.length),
      );
      _error = null;
    });
  }

  Future<void> _scan() async {
    final code = (await _controller.scanner.scan(context))?.trim();
    if (!mounted || code == null || code.isEmpty) return;
    setState(() {
      _field.value = TextEditingValue(
        text: code,
        selection: TextSelection.collapsed(offset: code.length),
      );
    });
    await _submit();
  }

  Future<void> _submit() async {
    if (_busy) return;
    setState(() {
      _busy = true;
      _error = null;
    });
    final result = await _controller.importLink(_field.text);
    // При успехе корневой экран уже сменился и это окно закрыто.
    if (!mounted) return;
    setState(() {
      _busy = false;
      _error = switch (result) {
        ImportFailed(:final message) => message,
        ImportDone() || ImportCancelled() => null,
      };
    });
  }

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    final canScan = BraidScope.of(context).caps.qrScan;
    return Scaffold(
      body: SafeArea(
        child: Center(
          child: SingleChildScrollView(
            padding: const EdgeInsets.all(24),
            child: ConstrainedBox(
              constraints: const BoxConstraints(maxWidth: 440),
              child: Column(
                mainAxisSize: MainAxisSize.min,
                crossAxisAlignment: CrossAxisAlignment.stretch,
                children: [
                  Icon(
                    Icons.vpn_lock_outlined,
                    size: 64,
                    color: theme.colorScheme.primary,
                  ),
                  const SizedBox(height: 16),
                  Text(
                    'Вставь ссылку или отсканируй QR',
                    textAlign: TextAlign.center,
                    style: theme.textTheme.titleLarge,
                  ),
                  const SizedBox(height: 8),
                  Text(
                    'Ссылку вида braid://… даёт тот, кто выдаёт тебе доступ.',
                    textAlign: TextAlign.center,
                    style: theme.textTheme.bodyMedium?.copyWith(
                      color: theme.colorScheme.onSurfaceVariant,
                    ),
                  ),
                  const SizedBox(height: 24),
                  TextField(
                    key: const Key('link-field'),
                    controller: _field,
                    minLines: 1,
                    maxLines: 3,
                    keyboardType: TextInputType.url,
                    textInputAction: TextInputAction.done,
                    autocorrect: false,
                    enableSuggestions: false,
                    decoration: InputDecoration(
                      labelText: 'Ссылка $appName',
                      hintText: 'braid://…',
                      errorText: _error,
                      errorMaxLines: 3,
                    ),
                    onChanged: (_) {
                      if (_error != null) setState(() => _error = null);
                    },
                    onSubmitted: (_) => _submit(),
                  ),
                  const SizedBox(height: 12),
                  Wrap(
                    spacing: 12,
                    runSpacing: 12,
                    alignment: WrapAlignment.center,
                    children: [
                      OutlinedButton.icon(
                        key: const Key('paste-button'),
                        onPressed: _busy ? null : _paste,
                        icon: const Icon(Icons.content_paste),
                        label: const Text('Вставить из буфера'),
                      ),
                      if (canScan)
                        OutlinedButton.icon(
                          key: const Key('scan-button'),
                          onPressed: _busy ? null : _scan,
                          icon: const Icon(Icons.qr_code_scanner),
                          label: const Text('Сканировать QR'),
                        ),
                    ],
                  ),
                  const SizedBox(height: 16),
                  FilledButton(
                    key: const Key('add-button'),
                    onPressed: _busy ? null : _submit,
                    child: _busy
                        ? const SizedBox.square(
                            dimension: 20,
                            child: CircularProgressIndicator(strokeWidth: 2),
                          )
                        : const Text('Добавить'),
                  ),
                  const SizedBox(height: 40),
                  // Подключиться нечем, пока не добавлена подписка (ист. 4).
                  const FilledButton(
                    onPressed: null,
                    child: Text('Подключить'),
                  ),
                  const SizedBox(height: 8),
                  Text(
                    'Подключиться можно, когда добавлена подписка',
                    textAlign: TextAlign.center,
                    style: theme.textTheme.bodySmall?.copyWith(
                      color: theme.colorScheme.onSurfaceVariant,
                    ),
                  ),
                ],
              ),
            ),
          ),
        ),
      ),
    );
  }
}

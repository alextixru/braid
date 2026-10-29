import 'package:braid/controller/braid_scope.dart';
import 'package:braid/models/commands.dart';
import 'package:braid/util/rule_input.dart';
import 'package:braid/widgets/section_header.dart';
import 'package:flutter/material.dart';

/// «Мои правила»: одно поле для домена и подсети. Ввод приводится к тому виду,
/// который примет ядро, — подсказка «Добавится: …» видна до нажатия; ошибка
/// показывается на месте, а введённое остаётся (ист. 50, 51).
class UserRulesSection extends StatefulWidget {
  const UserRulesSection({super.key});

  @override
  State<UserRulesSection> createState() => _UserRulesSectionState();
}

class _UserRulesSectionState extends State<UserRulesSection> {
  final _field = TextEditingController();
  String? _coreError;
  bool _submitted = false;
  bool _busy = false;

  @override
  void dispose() {
    _field.dispose();
    super.dispose();
  }

  Future<void> _add() async {
    if (_busy) return;
    setState(() {
      _submitted = true;
      _busy = true;
    });
    final error = await BraidScope.read(context).addRule(_field.text);
    if (!mounted) return;
    setState(() {
      _busy = false;
      _coreError = error;
      if (error == null) {
        _field.clear();
        _submitted = false;
      }
    });
  }

  @override
  Widget build(BuildContext context) {
    final controller = BraidScope.of(context);
    final rules = controller.state.rules;
    final theme = Theme.of(context);

    String? helper;
    String? parseError;
    if (_field.text.trim().isNotEmpty) {
      try {
        final rule = RuleInput.classify(_field.text);
        helper = switch (rule.kind) {
          RuleKind.domain => 'Добавится домен: ${rule.value}',
          RuleKind.subnet => 'Добавится подсеть: ${rule.value}',
        };
      } on RuleInputException catch (e) {
        parseError = e.message;
      }
    } else if (_submitted) {
      parseError = 'Введи домен или подсеть';
    }

    return Column(
      crossAxisAlignment: CrossAxisAlignment.start,
      children: [
        const SectionHeader(
          'Мои правила',
          subtitle:
              'Свои домены и подсети идут через VPN поверх общих наборов.',
        ),
        TextField(
          key: const Key('rule-field'),
          controller: _field,
          keyboardType: TextInputType.url,
          textInputAction: TextInputAction.done,
          autocorrect: false,
          enableSuggestions: false,
          decoration: InputDecoration(
            labelText: 'Домен или подсеть',
            hintText: 'example.com, *.example.com, 10.0.0.0/8',
            helperText: helper,
            errorText: _coreError ?? (_submitted ? parseError : null),
            errorMaxLines: 3,
          ),
          onChanged: (_) => setState(() => _coreError = null),
          onSubmitted: (_) => _add(),
        ),
        const SizedBox(height: 8),
        Align(
          alignment: Alignment.centerRight,
          child: FilledButton.tonal(
            key: const Key('rule-add-button'),
            onPressed: _busy ? null : _add,
            child: const Text('Добавить'),
          ),
        ),
        const SizedBox(height: 8),
        if (rules.userDomains.isEmpty && rules.userCidrs.isEmpty)
          Padding(
            padding: const EdgeInsets.symmetric(vertical: 8),
            child: Text(
              'Своих правил пока нет',
              style: theme.textTheme.bodyMedium?.copyWith(
                color: theme.colorScheme.onSurfaceVariant,
              ),
            ),
          )
        else
          Card(
            child: Column(
              children: [
                for (final domain in rules.userDomains)
                  ListTile(
                    key: Key('user-domain-$domain'),
                    leading: const Icon(Icons.language),
                    title: Text(domain),
                    trailing: IconButton(
                      key: Key('remove-domain-$domain'),
                      tooltip: 'Удалить',
                      icon: const Icon(Icons.delete_outline),
                      onPressed: () => controller.applyRules(
                        RulesChange.removeUserDomain(domain),
                      ),
                    ),
                  ),
                for (final cidr in rules.userCidrs)
                  ListTile(
                    key: Key('user-cidr-$cidr'),
                    leading: const Icon(Icons.lan_outlined),
                    title: Text(cidr),
                    trailing: IconButton(
                      key: Key('remove-cidr-$cidr'),
                      tooltip: 'Удалить',
                      icon: const Icon(Icons.delete_outline),
                      onPressed: () => controller.applyRules(
                        RulesChange.removeUserCidr(cidr),
                      ),
                    ),
                  ),
              ],
            ),
          ),
      ],
    );
  }
}

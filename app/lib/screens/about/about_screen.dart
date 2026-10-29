/// Экран «О программе» (ист. 21, R34i): имя и версии, компоненты и лицензии из
/// `THIRD_PARTY.md`, ограничение лицензии csqtt-core — сразу под названием.
library;

import 'package:braid/app.dart' show appName;
import 'package:braid/controller/braid_scope.dart';
import 'package:braid/screens/about/third_party.dart';
import 'package:braid/widgets/banner_card.dart';
import 'package:braid/widgets/page_body.dart';
import 'package:braid/widgets/section_header.dart';
import 'package:flutter/material.dart';

/// Подключается как `AppExtensions.about`.
Widget buildAboutScreen(BuildContext context) => const AboutScreen();

/// Копия `THIRD_PARTY.md` из корня репозитория (совпадение проверяет тест).
const String thirdPartyAsset = 'assets/third_party.md';

class AboutScreen extends StatefulWidget {
  const AboutScreen({super.key});

  @override
  State<AboutScreen> createState() => _AboutScreenState();
}

class _AboutScreenState extends State<AboutScreen> {
  Future<ThirdPartyDoc>? _doc;

  @override
  void didChangeDependencies() {
    super.didChangeDependencies();
    // Без кэша бандла: экран открывают редко, а файл мал, зато чтение не
    // зависит от того, что осталось от прошлого открытия.
    _doc ??= DefaultAssetBundle.of(context)
        .loadString(thirdPartyAsset, cache: false)
        .then(ThirdPartyDoc.parse);
  }

  @override
  Widget build(BuildContext context) {
    final controller = BraidScope.of(context);
    final theme = Theme.of(context);
    return Scaffold(
      appBar: AppBar(title: const Text('О программе')),
      body: PageBody(
        children: [
          Row(
            children: [
              Icon(
                Icons.hub_outlined,
                size: 48,
                color: theme.colorScheme.primary,
              ),
              const SizedBox(width: 16),
              Expanded(
                child: Column(
                  crossAxisAlignment: CrossAxisAlignment.start,
                  children: [
                    Text(
                      appName,
                      key: const Key('about-name'),
                      style: theme.textTheme.headlineSmall,
                    ),
                    Text('Версия ${controller.clientVersion}'),
                    Text(
                      'Ядро ${controller.coreVersion}',
                      style: theme.textTheme.bodySmall?.copyWith(
                        color: theme.colorScheme.onSurfaceVariant,
                      ),
                    ),
                  ],
                ),
              ),
            ],
          ),
          const SizedBox(height: 16),
          FutureBuilder<ThirdPartyDoc>(
            future: _doc,
            builder: (context, snapshot) {
              if (snapshot.hasError) {
                return Text(
                  'Не удалось прочитать список компонентов',
                  style: theme.textTheme.bodyMedium?.copyWith(
                    color: theme.colorScheme.error,
                  ),
                );
              }
              final doc = snapshot.data;
              // Файл в пакете приложения читается мгновенно — индикатор
              // загрузки здесь только мигал бы.
              if (doc == null) return const SizedBox.shrink();
              return _Components(doc: doc);
            },
          ),
          const SizedBox(height: 16),
          Card(
            child: ListTile(
              key: const Key('about-licenses'),
              leading: const Icon(Icons.description_outlined),
              title: const Text('Полные тексты лицензий'),
              subtitle: const Text(
                'Пакеты Flutter и Dart, из которых собрано приложение',
              ),
              trailing: const Icon(Icons.chevron_right),
              onTap: () => showLicensePage(
                context: context,
                applicationName: appName,
                applicationVersion: controller.clientVersion,
              ),
            ),
          ),
        ],
      ),
    );
  }
}

/// Компоненты и лицензии: всё, что есть в `THIRD_PARTY.md`.
class _Components extends StatelessWidget {
  const _Components({required this.doc});

  final ThirdPartyDoc doc;

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    final notices = _noncommercialNotices(doc);
    return Column(
      key: const Key('about-components'),
      crossAxisAlignment: CrossAxisAlignment.start,
      children: [
        for (final notice in notices) ...[
          BannerCard(
            key: const Key('about-noncommercial'),
            icon: Icons.gavel_outlined,
            title: 'Лицензия ${plainText(notice.license)}',
            subtitle: plainText(notice.note),
            isProblem: true,
          ),
          const SizedBox(height: 8),
        ],
        for (final paragraph in doc.intro)
          Text(
            plainText(paragraph),
            style: theme.textTheme.bodySmall?.copyWith(
              color: theme.colorScheme.onSurfaceVariant,
            ),
          ),
        for (final section in doc.sections) ...[
          SectionHeader(plainText(section.title)),
          for (final block in section.blocks)
            switch (block) {
              ThirdPartyText(:final text) => Text(
                plainText(text),
                style: theme.textTheme.bodySmall,
              ),
              ThirdPartyTable() => _Table(table: block),
            },
        ],
      ],
    );
  }

  /// Лицензии с запретом коммерческого использования и то, что файл о них
  /// говорит: первая такая строка с примечанием.
  static List<_Notice> _noncommercialNotices(ThirdPartyDoc doc) {
    for (final section in doc.sections) {
      for (final block in section.blocks) {
        if (block is! ThirdPartyTable) continue;
        final license = _column(block, 'лиценз');
        final note = _column(block, 'примечан');
        if (license == null || note == null) continue;
        for (final row in block.rows) {
          if (row.length > license &&
              row.length > note &&
              isNoncommercial(row[license]) &&
              row[note].isNotEmpty) {
            return [_Notice(license: row[license], note: row[note])];
          }
        }
      }
    }
    return const [];
  }
}

class _Notice {
  const _Notice({required this.license, required this.note});

  final String license;
  final String note;
}

/// Индекс столбца таблицы по началу заголовка (без учёта регистра).
int? _column(ThirdPartyTable table, String prefix) {
  for (var i = 0; i < table.header.length; i++) {
    if (table.header[i].toLowerCase().startsWith(prefix)) return i;
  }
  return null;
}

/// Таблица как список карточек: имя, лицензия, остальные столбцы подписями.
class _Table extends StatelessWidget {
  const _Table({required this.table});

  final ThirdPartyTable table;

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    final licenseAt = _column(table, 'лиценз');
    return Card(
      child: Column(
        children: [
          for (var r = 0; r < table.rows.length; r++) ...[
            if (r > 0) const Divider(height: 1),
            Builder(
              builder: (context) {
                final row = table.rows[r];
                final license = licenseAt != null && licenseAt < row.length
                    ? row[licenseAt]
                    : null;
                final restricted = license != null && isNoncommercial(license);
                return ListTile(
                  title: Text(plainText(row.first)),
                  subtitle: Column(
                    crossAxisAlignment: CrossAxisAlignment.start,
                    children: [
                      if (license != null)
                        Text(
                          plainText(license),
                          style: theme.textTheme.labelLarge?.copyWith(
                            color: restricted ? theme.colorScheme.error : null,
                          ),
                        ),
                      for (var c = 1; c < row.length; c++)
                        if (c != licenseAt && row[c].isNotEmpty)
                          Text(
                            '${plainText(table.header[c])}: ${plainText(row[c])}',
                          ),
                    ],
                  ),
                );
              },
            ),
          ],
        ],
      ),
    );
  }
}

package org.braid.vpn

import android.content.pm.ApplicationInfo
import android.content.pm.PackageManager

/**
 * Установленное приложение для правил «по приложениям» (ист. 42, 43). [system] — флаг
 * `FLAG_SYSTEM`: окно может отделить системные пакеты от поставленных пользователем.
 */
data class InstalledApp(val packageName: String, val label: String, val system: Boolean)

/**
 * Ответ `installedApps` канала `org.braid/vpn`: список словарей
 * `{"package": …, "label": …, "system": Boolean}` — `package` и `label` читает Dart
 * (`InstalledApp` в `vpn_channel.dart`), `system` — флаг системного приложения.
 * Чтение из `PackageManager` — [load] (только на устройстве); порядок, чистка и форма —
 * чистая логика [sorted] и [forDart], её проверяет unit-тест.
 */
object InstalledApps {
    /**
     * Все приложения устройства. С Android 11 `PackageManager` показывает чужие пакеты
     * только при `QUERY_ALL_PACKAGES` в манифесте.
     */
    fun load(pm: PackageManager): List<InstalledApp> {
        @Suppress("DEPRECATION")
        val infos = pm.getInstalledApplications(0)
        return infos.map { info ->
            InstalledApp(
                packageName = info.packageName,
                label = info.loadLabel(pm).toString(),
                system = (info.flags and ApplicationInfo.FLAG_SYSTEM) != 0,
            )
        }
    }

    /** Список в форме для Dart: см. [sorted] про порядок. */
    fun forDart(apps: List<InstalledApp>, selfPackage: String): List<Map<String, Any>> =
        sorted(apps, selfPackage).map {
            mapOf("package" to it.packageName, "label" to it.label, "system" to it.system)
        }

    /**
     * Порядок и чистка: без самого Braid ([selfPackage] — в туннель он не ходит, см.
     * `VpnInterfaceSpec`), без пустых пакетов и дублей (остаётся первый), имя без краевых
     * пробелов, пустое имя — сам пакет; сначала пользовательские приложения, затем
     * системные, внутри — по имени без учёта регистра, при равных именах — по пакету.
     */
    fun sorted(apps: List<InstalledApp>, selfPackage: String): List<InstalledApp> =
        apps.asSequence()
            .filter { it.packageName.isNotBlank() && it.packageName != selfPackage }
            .map { app -> app.copy(label = app.label.trim().ifEmpty { app.packageName }) }
            .distinctBy { it.packageName }
            .sortedWith(compareBy<InstalledApp>({ it.system }, { it.label.lowercase() }, { it.packageName }))
            .toList()
}

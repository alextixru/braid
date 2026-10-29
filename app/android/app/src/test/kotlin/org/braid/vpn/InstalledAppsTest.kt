package org.braid.vpn

import org.junit.Assert.assertEquals
import org.junit.Test

/**
 * Список приложений для правил «по приложениям» (`installedApps` канала `org.braid/vpn`):
 * без Braid, без дублей, сначала пользовательские, по имени; форма словарей — как ждёт Dart.
 */
class InstalledAppsTest {
    private val self = "org.braid.vpn"

    @Test
    fun userAppsFirstThenSystemEachByLabelIgnoringCase() {
        val apps = listOf(
            InstalledApp("com.android.chrome", "Chrome", system = true),
            InstalledApp("org.telegram.messenger", "telegram", system = false),
            InstalledApp("com.bank.app", "Bank", system = false),
            InstalledApp("com.android.settings", "Settings", system = true),
            InstalledApp("ru.yandex.maps", "Яндекс Карты", system = false),
        )
        assertEquals(
            listOf(
                "com.bank.app",
                "org.telegram.messenger",
                "ru.yandex.maps",
                "com.android.chrome",
                "com.android.settings",
            ),
            InstalledApps.sorted(apps, self).map { it.packageName },
        )
    }

    @Test
    fun equalLabelsAreOrderedByPackage() {
        val apps = listOf(
            InstalledApp("com.b", "Same", system = false),
            InstalledApp("com.a", "Same", system = false),
        )
        assertEquals(listOf("com.a", "com.b"), InstalledApps.sorted(apps, self).map { it.packageName })
    }

    @Test
    fun braidItselfBlankPackagesDuplicatesAndBlankLabelsAreCleaned() {
        val apps = listOf(
            InstalledApp(self, "Braid", system = false),
            InstalledApp("com.x", "  X  ", system = false),
            InstalledApp("com.x", "X again", system = false),
            InstalledApp("com.nolabel", "   ", system = false),
            InstalledApp("", "Ghost", system = false),
        )
        val sorted = InstalledApps.sorted(apps, self)
        assertEquals(listOf("com.nolabel", "com.x"), sorted.map { it.packageName })
        assertEquals("com.nolabel", sorted[0].label)
        assertEquals("X", sorted[1].label)
    }

    @Test
    fun dartShapeHasPackageLabelAndSystemFlag() {
        val maps = InstalledApps.forDart(listOf(InstalledApp("com.x", "X", system = true)), self)
        assertEquals(listOf(mapOf("package" to "com.x", "label" to "X", "system" to true)), maps)
    }
}

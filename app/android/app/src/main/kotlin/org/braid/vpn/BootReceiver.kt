package org.braid.vpn

import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent

/**
 * Старт после перезагрузки (ист. 55), если включён автозапуск (флаг кладёт
 * `BraidVpnService.setAutostart` по `Platform::set_autostart`). Подключаться ли —
 * решает ядро (`Core::resume`: была ли сессия активна); если нет — сервис уходит сам.
 * Always-on система поднимает без нас (`SERVICE_INTERFACE`).
 */
class BootReceiver : BroadcastReceiver() {
    override fun onReceive(context: Context, intent: Intent) {
        if (intent.action != Intent.ACTION_BOOT_COMPLETED) return
        val prefs = context.getSharedPreferences(BraidVpnService.PREFS, Context.MODE_PRIVATE)
        if (!prefs.getBoolean(BraidVpnService.PREF_AUTOSTART, false)) return
        BraidVpnService.start(context, BraidVpnService.ACTION_RESUME)
    }
}

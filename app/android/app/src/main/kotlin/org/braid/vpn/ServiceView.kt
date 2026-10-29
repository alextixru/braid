package org.braid.vpn

import org.json.JSONObject

/**
 * Что показывает постоянное уведомление сервиса — JSON `updateNotification` из Rust
 * (`braid_platform::android::notification::NotificationView`).
 */
data class ServiceView(
    val state: String,
    val text: String,
    val detail: String?,
    val engine: String?,
    val mode: String,
    val nextMode: String,
    val nextLabel: String,
    val canCycle: Boolean,
) {
    companion object {
        fun fromJson(json: String): ServiceView {
            val o = JSONObject(json)
            return ServiceView(
                state = o.optString("state"),
                text = o.optString("text"),
                detail = o.optStringOrNull("detail"),
                engine = o.optStringOrNull("engine"),
                mode = o.optString("mode", "auto"),
                nextMode = o.optString("next_mode", "auto"),
                nextLabel = o.optString("next_label"),
                canCycle = o.optBoolean("can_cycle", false),
            )
        }

        /** Пока ядро не прислало первое состояние. */
        fun starting(): ServiceView =
            ServiceView("starting", "Запуск…", null, null, "auto", "auto", "Авто", false)

        private fun JSONObject.optStringOrNull(key: String): String? =
            if (has(key) && !isNull(key)) getString(key) else null
    }
}

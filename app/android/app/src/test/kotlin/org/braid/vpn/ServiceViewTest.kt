package org.braid.vpn

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/** Уведомление сервиса (ист. 57): JSON `updateNotification` из Rust (`NotificationView`). */
class ServiceViewTest {
    @Test
    fun readsTheFieldsRustWrites() {
        val view = ServiceView.fromJson(
            """{"state":"connected","text":"Подключено · AWG","detail":"Режим: Авто","engine":"awg",""" +
                """"mode":"auto","next_mode":"awg","next_label":"AWG","can_cycle":true}""",
        )
        assertEquals("connected", view.state)
        assertEquals("Подключено · AWG", view.text)
        assertEquals("Режим: Авто", view.detail)
        assertEquals("awg", view.engine)
        assertEquals("auto", view.mode)
        assertEquals("awg", view.nextMode)
        assertEquals("AWG", view.nextLabel)
        assertTrue(view.canCycle)
    }

    @Test
    fun nullDetailAndEngineStayNullAndActionCanBeHidden() {
        val view = ServiceView.fromJson(
            """{"state":"needs_link","text":"Нужна ссылка подписки","detail":null,"engine":null,""" +
                """"mode":"auto","next_mode":"auto","next_label":"Авто","can_cycle":false}""",
        )
        assertNull(view.detail)
        assertNull(view.engine)
        assertFalse(view.canCycle)
    }

    @Test
    fun startingViewBeforeTheFirstState() {
        val view = ServiceView.starting()
        assertEquals("starting", view.state)
        assertFalse(view.canCycle)
    }
}

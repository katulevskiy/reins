package dev.rewarden.android.platform

import android.Manifest
import android.content.ContentUris
import android.content.ContentValues
import android.content.Context
import android.content.pm.PackageManager
import android.database.Cursor
import android.net.Uri
import android.provider.CalendarContract
import android.provider.ContactsContract
import android.provider.Telephony
import android.telephony.SmsManager
import android.text.format.DateUtils
import androidx.core.content.ContextCompat
import dev.rewarden.android.BuildConfig
import dev.rewarden.core.DeviceBridge
import dev.rewarden.core.DeviceContact
import dev.rewarden.core.DeviceEvent
import dev.rewarden.core.ForeignException
import dev.rewarden.core.NewDeviceEvent
import dev.rewarden.core.SmsMessage
import dev.rewarden.core.SmsThread
import java.util.TimeZone

/**
 * This phone's own calendar, contacts and text messages, read and written through Android's content providers. Nothing
 * here decides what an AI may see: every call comes from the core after the user approved it, and each fails with
 * "needs user interaction" while the matching Android permission is not granted.
 */
class PhoneBridge(private val context: Context) : DeviceBridge {
    private val resolver get() = context.contentResolver

    override fun services(): List<String> = SERVICES

    override fun permitted(service: String): Boolean = offers(service) && permissionsOf(service).all { granted(context, it) }

    private fun need(service: String) {
        if (!permitted(service)) throw ForeignException.NeedsUserInteraction()
    }

    /** Runs [block] and turns a revoked permission into "needs user interaction" and anything else into a message. */
    private inline fun <T> guarded(block: () -> T): T = try {
        block()
    } catch (e: SecurityException) {
        throw ForeignException.NeedsUserInteraction()
    } catch (e: ForeignException) {
        throw e
    } catch (e: Exception) {
        throw ForeignException.Failed("The phone could not do that: ${e.javaClass.simpleName}")
    }

    // ---- calendar --------------------------------------------------------------------------------------------

    override fun calendarEvents(from: Long, to: Long, query: String?, limit: UInt): List<DeviceEvent> = guarded {
        need(DEVICE_CALENDAR)
        val projection = arrayOf(
            CalendarContract.Instances.EVENT_ID,
            CalendarContract.Instances.CALENDAR_ID,
            CalendarContract.Instances.TITLE,
            CalendarContract.Instances.BEGIN,
            CalendarContract.Instances.END,
            CalendarContract.Instances.ALL_DAY,
            CalendarContract.Instances.EVENT_LOCATION,
            CalendarContract.Instances.DESCRIPTION,
            CalendarContract.Instances.CALENDAR_DISPLAY_NAME,
        )
        val builder = CalendarContract.Instances.CONTENT_SEARCH_URI.buildUpon()
        ContentUris.appendId(builder, from * 1000)
        ContentUris.appendId(builder, to * 1000)
        val uri = if (query.isNullOrBlank()) {
            CalendarContract.Instances.CONTENT_URI.buildUpon().also {
                ContentUris.appendId(it, from * 1000)
                ContentUris.appendId(it, to * 1000)
            }.build()
        } else {
            builder.appendPath(query).build()
        }
        val events = ArrayList<DeviceEvent>()
        resolver.query(uri, projection, null, null, "${CalendarContract.Instances.BEGIN} ASC")?.use { c ->
            while (c.moveToNext() && events.size < limit.toInt()) {
                val allDay = c.getInt(5) != 0
                events += DeviceEvent(
                    id = c.getLong(0).toString(),
                    calendarId = c.getLong(1).toString(),
                    calendar = c.getString(8).orEmpty(),
                    title = c.getString(2).orEmpty(),
                    // All-day events are stored as UTC midnight, which is what the date means.
                    start = c.getLong(3) / 1000,
                    end = c.getLong(4) / 1000,
                    allDay = allDay,
                    location = c.getString(6).orEmpty(),
                    description = c.getString(7).orEmpty(),
                )
            }
        }
        events
    }

    override fun calendarCreate(event: NewDeviceEvent): String = guarded {
        need(DEVICE_CALENDAR)
        val calendarId = writableCalendar() ?: throw ForeignException.Failed("This phone has no calendar that can be written to")
        val values = ContentValues().apply {
            put(CalendarContract.Events.CALENDAR_ID, calendarId)
            put(CalendarContract.Events.TITLE, event.title)
            put(CalendarContract.Events.DTSTART, event.start * 1000)
            put(CalendarContract.Events.DTEND, event.end * 1000)
            put(CalendarContract.Events.ALL_DAY, if (event.allDay) 1 else 0)
            put(CalendarContract.Events.EVENT_TIMEZONE, if (event.allDay) "UTC" else TimeZone.getDefault().id)
            if (event.location.isNotEmpty()) put(CalendarContract.Events.EVENT_LOCATION, event.location)
            if (event.description.isNotEmpty()) put(CalendarContract.Events.DESCRIPTION, event.description)
        }
        val created = resolver.insert(CalendarContract.Events.CONTENT_URI, values)
            ?: throw ForeignException.Failed("The calendar did not accept the event")
        ContentUris.parseId(created).toString()
    }

    /** The calendar new events go to: the primary one the user owns, else any they can write to. */
    private fun writableCalendar(): Long? {
        val projection = arrayOf(CalendarContract.Calendars._ID, CalendarContract.Calendars.IS_PRIMARY)
        val selection = "${CalendarContract.Calendars.CALENDAR_ACCESS_LEVEL} >= ${CalendarContract.Calendars.CAL_ACCESS_CONTRIBUTOR} AND ${CalendarContract.Calendars.VISIBLE} = 1"
        var first: Long? = null
        resolver.query(CalendarContract.Calendars.CONTENT_URI, projection, selection, null, null)?.use { c ->
            while (c.moveToNext()) {
                if (first == null) first = c.getLong(0)
                if (c.getInt(1) != 0) return c.getLong(0)
            }
        }
        return first
    }

    // ---- contacts --------------------------------------------------------------------------------------------

    override fun contactsSearch(query: String, limit: UInt): List<DeviceContact> = guarded {
        need(DEVICE_CONTACTS)
        val ids = LinkedHashSet<Long>()
        for (uri in listOf(
            ContactsContract.Contacts.CONTENT_FILTER_URI,
            ContactsContract.CommonDataKinds.Phone.CONTENT_FILTER_URI,
            ContactsContract.CommonDataKinds.Email.CONTENT_FILTER_URI,
        )) {
            val column = if (uri == ContactsContract.Contacts.CONTENT_FILTER_URI) ContactsContract.Contacts._ID else ContactsContract.Data.CONTACT_ID
            resolver.query(Uri.withAppendedPath(uri, Uri.encode(query)), arrayOf(column), null, null, null)?.use { c ->
                while (c.moveToNext() && ids.size < limit.toInt()) ids += c.getLong(0)
            }
        }
        ids.take(limit.toInt()).map { contact(it) }
    }

    private fun contact(id: Long): DeviceContact {
        var name = ""
        var organization = ""
        val emails = ArrayList<String>()
        val phones = ArrayList<String>()
        val projection = arrayOf(ContactsContract.Data.MIMETYPE, ContactsContract.Data.DATA1, ContactsContract.Data.DISPLAY_NAME)
        resolver.query(ContactsContract.Data.CONTENT_URI, projection, "${ContactsContract.Data.CONTACT_ID} = ?", arrayOf(id.toString()), null)?.use { c ->
            while (c.moveToNext()) {
                val value = c.getString(1).orEmpty()
                if (name.isEmpty()) name = c.getString(2).orEmpty()
                when (c.getString(0)) {
                    ContactsContract.CommonDataKinds.Email.CONTENT_ITEM_TYPE -> if (value.isNotEmpty()) emails += value
                    ContactsContract.CommonDataKinds.Phone.CONTENT_ITEM_TYPE -> if (value.isNotEmpty()) phones += value
                    ContactsContract.CommonDataKinds.Organization.CONTENT_ITEM_TYPE -> if (organization.isEmpty()) organization = value
                }
            }
        }
        return DeviceContact(id.toString(), name, organization, emails.distinct(), phones.distinct())
    }

    // ---- text messages ---------------------------------------------------------------------------------------

    override fun smsThreads(limit: UInt): List<SmsThread> = guarded {
        need(SMS)
        val projection = arrayOf(Telephony.Sms.THREAD_ID, Telephony.Sms.ADDRESS, Telephony.Sms.BODY, Telephony.Sms.DATE)
        val threads = LinkedHashMap<Long, SmsThread>()
        resolver.query(Telephony.Sms.CONTENT_URI, projection, null, null, "${Telephony.Sms.DATE} DESC")?.use { c ->
            // Newest first, so the first row of a conversation is its latest message.
            while (c.moveToNext() && threads.size < limit.toInt() && c.position < SCAN_LIMIT) {
                val thread = c.getLong(0)
                if (thread in threads) continue
                val address = c.getString(1).orEmpty()
                threads[thread] = SmsThread(thread.toString(), address, contactName(address), c.getString(2).orEmpty(), c.getLong(3) / 1000)
            }
        }
        threads.values.toList()
    }

    override fun smsMessages(thread: String, limit: UInt): List<SmsMessage> = guarded {
        need(SMS)
        val id = thread.toLongOrNull() ?: throw ForeignException.Failed("That is not a conversation id")
        val projection = arrayOf(Telephony.Sms._ID, Telephony.Sms.ADDRESS, Telephony.Sms.BODY, Telephony.Sms.DATE, Telephony.Sms.TYPE)
        val messages = ArrayList<SmsMessage>()
        resolver.query(Telephony.Sms.CONTENT_URI, projection, "${Telephony.Sms.THREAD_ID} = ?", arrayOf(id.toString()), "${Telephony.Sms.DATE} DESC")?.use { c ->
            while (c.moveToNext() && messages.size < limit.toInt()) {
                val type = c.getInt(4)
                messages += SmsMessage(
                    id = c.getLong(0).toString(),
                    address = c.getString(1).orEmpty(),
                    body = c.getString(2).orEmpty(),
                    date = c.getLong(3) / 1000,
                    outgoing = type != Telephony.Sms.MESSAGE_TYPE_INBOX,
                )
            }
        }
        messages
    }

    override fun smsSend(to: String, text: String) = guarded {
        need(SMS)
        val manager = context.getSystemService(SmsManager::class.java) ?: throw ForeignException.Failed("This phone cannot send text messages")
        val parts = manager.divideMessage(text)
        if (parts.size > 1) manager.sendMultipartTextMessage(to, null, parts, null, null) else manager.sendTextMessage(to, null, text, null, null)
    }

    /** The contact a number belongs to, when the contacts may be read. */
    private fun contactName(number: String): String {
        if (number.isEmpty() || !granted(context, Manifest.permission.READ_CONTACTS)) return ""
        val uri = Uri.withAppendedPath(ContactsContract.PhoneLookup.CONTENT_FILTER_URI, Uri.encode(number))
        return try {
            resolver.query(uri, arrayOf(ContactsContract.PhoneLookup.DISPLAY_NAME), null, null, null)?.use { c: Cursor -> if (c.moveToFirst()) c.getString(0).orEmpty() else "" }.orEmpty()
        } catch (e: RuntimeException) {
            ""
        }
    }

    companion object {
        const val DEVICE_CALENDAR = "device_calendar"
        const val DEVICE_CONTACTS = "device_contacts"
        const val SMS = "sms"
        private const val SCAN_LIMIT = 3_000

        /**
         * The on-phone integrations this build offers. The `play` build has no text messages: Google Play allows the
         * SMS permissions only to default SMS apps, so its manifest does not ask for them.
         */
        val SERVICES: List<String> = listOfNotNull(DEVICE_CALENDAR, DEVICE_CONTACTS, SMS.takeIf { BuildConfig.HAS_SMS })

        /** Whether this build offers [service]; every integration that does not live on the phone is offered. */
        fun offers(service: String): Boolean = service != SMS || BuildConfig.HAS_SMS

        /** The Android permissions each on-phone service needs. */
        fun permissionsOf(service: String): List<String> = when (service) {
            DEVICE_CALENDAR -> listOf(Manifest.permission.READ_CALENDAR, Manifest.permission.WRITE_CALENDAR)
            DEVICE_CONTACTS -> listOf(Manifest.permission.READ_CONTACTS)
            SMS -> listOf(Manifest.permission.READ_SMS, Manifest.permission.SEND_SMS)
            else -> emptyList()
        }

        fun granted(context: Context, permission: String): Boolean =
            ContextCompat.checkSelfPermission(context, permission) == PackageManager.PERMISSION_GRANTED
    }
}

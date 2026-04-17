/**
 * mm:ss countdown — used by the recording timer in Timeline & StatusBar.
 */
export function formatCountdown(secs: number): string {
  const m = Math.floor(secs / 60);
  const s = secs % 60;
  return `${m}:${s.toString().padStart(2, "0")}`;
}

/**
 * "Today" / "Yesterday" / "Apr 14" — relative date label for a Date object.
 * Falls back to long format if more than two days in the past.
 */
export function formatRelativeDate(d: Date): string {
  const today = new Date();
  if (d.toDateString() === today.toDateString()) return "Today";
  const yesterday = new Date(today);
  yesterday.setDate(yesterday.getDate() - 1);
  if (d.toDateString() === yesterday.toDateString()) return "Yesterday";
  return d.toLocaleDateString([], { month: "short", day: "numeric" });
}

/**
 * Same as formatRelativeDate but accepts the canonical date string format
 * "YYYY-MM-DD" used by the FilterPanel chip list, and adds the weekday.
 */
export function formatRelativeDateWithWeekday(dateStr: string): string {
  try {
    const d = new Date(dateStr + "T00:00:00");
    const today = new Date();
    if (d.toDateString() === today.toDateString()) return "Today";
    const yesterday = new Date(today);
    yesterday.setDate(yesterday.getDate() - 1);
    if (d.toDateString() === yesterday.toDateString()) return "Yesterday";
    return d.toLocaleDateString([], { weekday: "short", month: "short", day: "numeric" });
  } catch {
    return dateStr;
  }
}

/**
 * "Today 09:00 – 10:00" for an hour_key in the format "YYYY-MM-DDTHH".
 * The key is UTC-bucketed on the Rust side; we append `Z` before parsing
 * so the Date is anchored to the UTC instant and `.toLocaleTimeString()`
 * renders it in the user's local zone.
 */
export function formatHourRange(hourKey: string): string {
  try {
    const [date, hourStr] = hourKey.split("T");
    const hour = parseInt(hourStr, 10);
    const start = new Date(`${date}T${hour.toString().padStart(2, "0")}:00:00Z`);
    const end = new Date(start.getTime() + 3_600_000);
    const startTime = start.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });
    const endTime = end.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });
    return `${formatRelativeDate(start)} ${startTime} \u2013 ${endTime}`;
  } catch {
    return hourKey;
  }
}

/**
 * "09:30" for an ISO 8601 timestamp.
 */
export function formatTime(iso: string): string {
  try {
    return new Date(iso).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });
  } catch {
    return "";
  }
}

/**
 * "9:05 PM" for an epoch millisecond timestamp.
 */
export function formatSegmentTime(epochMs: number): string {
  try {
    return new Date(epochMs).toLocaleTimeString([], {
      hour: "numeric",
      minute: "2-digit",
    });
  } catch {
    return "";
  }
}

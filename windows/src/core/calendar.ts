// The Cal.com calendar's date math, as in CalcomCalendarView (macOS): a month
// cut into rows of 7 consecutive days starting on day 1 (no weekday
// alignment), shown half a month at a time: Q1 is the first two rows, Q2 the
// rest. Everything is in local time.

export type CalendarPage = { year: number; month: number; half: 1 | 2 };

/** The page that holds `date`: its month, Q2 from day 15 on. */
export function pageFor(date: Date): CalendarPage {
  return { year: date.getFullYear(), month: date.getMonth(), half: date.getDate() > 14 ? 2 : 1 };
}

/** "2026-10-02", the key bookings are grouped by. */
export function dayKey(date: Date): string {
  const pad = (n: number) => String(n).padStart(2, "0");
  return `${date.getFullYear()}-${pad(date.getMonth() + 1)}-${pad(date.getDate())}`;
}

/** The whole month in rows of 7 days from day 1; the last row is padded with null. */
export function monthRows(year: number, month: number): (Date | null)[][] {
  const days = new Date(year, month + 1, 0).getDate();
  const rows: (Date | null)[][] = [];
  for (let start = 1; start <= days; start += 7) {
    const row: (Date | null)[] = [];
    for (let d = start; d < start + 7; d++) row.push(d <= days ? new Date(year, month, d) : null);
    rows.push(row);
  }
  return rows;
}

/** The rows a page shows: the first two for Q1, the others for Q2. */
export function pageRows(page: CalendarPage): (Date | null)[][] {
  const rows = monthRows(page.year, page.month);
  return page.half === 1 ? rows.slice(0, 2) : rows.slice(2);
}

/** One half month back (-1) or forward (+1). */
export function stepPage(page: CalendarPage, direction: -1 | 1): CalendarPage {
  if (direction === 1) {
    if (page.half === 1) return { ...page, half: 2 };
    const next = new Date(page.year, page.month + 1, 1);
    return { year: next.getFullYear(), month: next.getMonth(), half: 1 };
  }
  if (page.half === 2) return { ...page, half: 1 };
  const prev = new Date(page.year, page.month - 1, 1);
  return { year: prev.getFullYear(), month: prev.getMonth(), half: 2 };
}

/** Bookings starting on the day of `key`, earliest first. */
export function bookingsOn<T extends { start?: unknown }>(bookings: readonly T[], key: string): T[] {
  return bookings
    .filter((b) => {
      const when = new Date(String(b.start));
      return !Number.isNaN(when.getTime()) && dayKey(when) === key;
    })
    .sort((a, b) => new Date(String(a.start)).getTime() - new Date(String(b.start)).getTime());
}

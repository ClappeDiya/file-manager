/**
 * Schedule library: named schedules and holiday calendars shared by sync
 * pairs and automation rules (`schedule:<id>` in their cron field) and
 * backup jobs (holiday calendars). Mirrors `schedule_library` in Rust.
 */
import { create } from "zustand";
import { tauriInvoke } from "@/hooks/use-tauri";

export interface NamedSchedule {
  id: string;
  name: string;
  description: string;
  /** `sec min hour day month weekday [year]`, UTC. */
  cron_expr: string;
  calendar_ids: string[];
  updated_at: string;
}

export interface HolidayCalendar {
  id: string;
  name: string;
  /** One-off dates, `YYYY-MM-DD`. */
  dates: string[];
  /** Yearly dates, `MM-DD`. */
  annual: string[];
  skip_weekends: boolean;
}

export interface PreviewRun {
  at: string;
  skipped_by: string | null;
}

/** Prefix of a cron field that refers to a named schedule. */
export const SCHEDULE_REF = "schedule:";
export const scheduleRef = (id: string) => `${SCHEDULE_REF}${id}`;
export const refId = (cron: string | null | undefined) => (cron?.trim().startsWith(SCHEDULE_REF) ? cron.trim().slice(SCHEDULE_REF.length) : null);

export const newSchedule = (): NamedSchedule => ({ id: "", name: "", description: "", cron_expr: "0 0 9 * * Mon-Fri", calendar_ids: [], updated_at: "" });
export const newCalendar = (): HolidayCalendar => ({ id: "", name: "", dates: [], annual: [], skip_weekends: false });

interface ScheduleState {
  schedules: NamedSchedule[];
  calendars: HolidayCalendar[];
  loaded: boolean;
  load: () => Promise<void>;
  saveSchedule: (s: NamedSchedule) => Promise<NamedSchedule>;
  deleteSchedule: (id: string) => Promise<void>;
  saveCalendar: (c: HolidayCalendar) => Promise<HolidayCalendar>;
  deleteCalendar: (id: string) => Promise<void>;
  importIcs: (id: string | null, name: string, path: string) => Promise<[HolidayCalendar, number]>;
  preview: (cronExpr: string, calendarIds?: string[], count?: number) => Promise<PreviewRun[]>;
}

export const useScheduleStore = create<ScheduleState>((set, get) => ({
  schedules: [],
  calendars: [],
  loaded: false,

  load: async () => {
    const [schedules, calendars] = await Promise.all([
      tauriInvoke<NamedSchedule[]>("schedules_list", {}, []),
      tauriInvoke<HolidayCalendar[]>("calendars_list", {}, []),
    ]);
    set({ schedules, calendars, loaded: true });
  },

  saveSchedule: async (schedule) => {
    const saved = await tauriInvoke<NamedSchedule>("schedule_save", { schedule });
    await get().load();
    return saved;
  },

  deleteSchedule: async (id) => {
    await tauriInvoke<void>("schedule_delete", { id });
    await get().load();
  },

  saveCalendar: async (calendar) => {
    const saved = await tauriInvoke<HolidayCalendar>("calendar_save", { calendar });
    await get().load();
    return saved;
  },

  deleteCalendar: async (id) => {
    await tauriInvoke<void>("calendar_delete", { id });
    await get().load();
  },

  importIcs: async (id, name, path) => {
    const r = await tauriInvoke<[HolidayCalendar, number]>("calendar_import_ics", { id, name, path });
    await get().load();
    return r;
  },

  preview: (cronExpr, calendarIds = [], count = 10) => tauriInvoke<PreviewRun[]>("schedule_preview", { cronExpr, calendarIds, count }, []),
}));

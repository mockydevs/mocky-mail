import { lazy } from 'react';

/**
 * The calendar and tasks screens are chunks of their own, so the mail list
 * doesn't wait for them. They start loading as soon as the app is up and
 * are normally there before they are opened.
 */
const loadCalendar = () => import('../features/calendar/CalendarView');
const loadTasks = () => import('../features/tasks/TasksView');

export const CalendarScreen = lazy(() => loadCalendar().then((m) => ({ default: m.CalendarView })));
export const TasksScreen = lazy(() => loadTasks().then((m) => ({ default: m.TasksView })));

export function preloadScreens(): void {
  loadCalendar().catch(() => undefined);
  loadTasks().catch(() => undefined);
}

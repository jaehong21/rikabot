import { clsx, type ClassValue } from "clsx";
import { twMerge } from "tailwind-merge";

import type { ThreadRecord } from "@/types/app";

export function cn(...inputs: ClassValue[]): string {
  return twMerge(clsx(inputs));
}

export type ThreadGroup = {
  label: string;
  threads: ThreadRecord[];
};

function dateKey(value: string): string {
  return new Date(value).toISOString().slice(0, 10);
}

export function groupThreads(threads: ThreadRecord[]): ThreadGroup[] {
  const now = new Date();
  const today = dateKey(now.toISOString());

  const yesterdayDate = new Date(now);
  yesterdayDate.setDate(now.getDate() - 1);
  const yesterday = dateKey(yesterdayDate.toISOString());

  const groups: ThreadGroup[] = [
    { label: "Today", threads: [] },
    { label: "Yesterday", threads: [] },
    { label: "Earlier", threads: [] },
  ];

  for (const thread of threads) {
    const key = dateKey(thread.updated_at);
    if (key === today) {
      groups[0].threads.push(thread);
      continue;
    }
    if (key === yesterday) {
      groups[1].threads.push(thread);
      continue;
    }
    groups[2].threads.push(thread);
  }

  return groups.filter((group) => group.threads.length > 0);
}

export function formatTime(value: number): string {
  const date = new Date(value);
  return date.toLocaleTimeString([], {
    hour: "2-digit",
    minute: "2-digit",
  });
}

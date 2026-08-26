/**
 * Where a run was taken.
 *
 * The tool is used by walking the house with a phone, and a stack of runs that
 * does not say which room each was taken in cannot answer the only question
 * worth asking of it: which room is the bad one. So the tag is chosen *before*
 * the run, from a row of chips beside the start control, rather than written
 * afterwards from memory.
 *
 * Deliberately not the same thing as a run's description. A description is
 * prose about one run; a location is a key that groups many, and the history
 * page filters on it exactly. Exact matching is also why the spellings are
 * reconciled here rather than left to whoever is typing — see `matchExisting`.
 */

import { fetchLocations } from './api';

/** Remembered per browser, so a walk through the house is a tap per room. */
const KEY = 'speedtest.location';

/**
 * As much of a tag as is kept.
 *
 * A location is a label, not a note: 64 characters is more than "Upstairs
 * landing, by the window" needs, and anything longer is a description that
 * belongs in the run's own description field.
 */
export const MAX_LOCATION = 64;

/**
 * A tag as it will be stored and compared.
 *
 * Whitespace is collapsed before the cap rather than after, so "Office␣␣" and
 * "Office" are one room rather than two — the filter is an exact match, and
 * two spellings of one place split its history in half without saying so.
 */
export function normaliseLocation(raw: string): string {
  return raw.replace(/\s+/g, ' ').trim().slice(0, MAX_LOCATION).trim();
}

/**
 * The spelling already in use for this place, if there is one.
 *
 * "office" and "Office" are one room to the person walking between them and
 * two rooms to an exact-match filter. The existing spelling wins, so a
 * carelessly typed tag joins the history it belongs to instead of starting a
 * near-identical second one.
 */
export function matchExisting(
  known: readonly string[],
  candidate: string,
): string | undefined {
  const want = candidate.toLowerCase();
  return known.find((k) => k.toLowerCase() === want);
}

/**
 * The chips to offer: everything the server knows about, plus the current
 * choice.
 *
 * The choice has to be included explicitly. A tag typed just now has not been
 * used by a stored run yet, so the server has never heard of it — and a chip
 * row that dropped the selected tag the moment it was chosen would be unusable.
 */
export function mergeLocations(known: readonly string[], selected: string): string[] {
  if (selected === '' || known.includes(selected)) return [...known];
  return [selected, ...known];
}

/** The remembered choice, or none. */
export function readStoredLocation(): string {
  try {
    return normaliseLocation(localStorage.getItem(KEY) ?? '');
  } catch {
    // Private browsing, or storage disabled. Not worth surfacing.
    return '';
  }
}

/** Remembers a choice, or forgets it when the choice is "no location". */
export function writeStoredLocation(value: string): void {
  try {
    if (value === '') localStorage.removeItem(KEY);
    else localStorage.setItem(KEY, value);
  } catch {
    // Storage unavailable; the choice lasts for this page only.
  }
}

/** Escapes text destined for innerHTML. Tags are typed by hand, on any client. */
function esc(s: string): string {
  return s
    .replace(/&/g, '&amp;')
    .replace(/</g, '&lt;')
    .replace(/>/g, '&gt;')
    .replace(/"/g, '&quot;');
}

export interface LocationPicker {
  /** The tag the next run should carry. Empty means "no location". */
  current(): string;
  /**
   * Shows or hides the whole control.
   *
   * A deployment that keeps no history has nowhere to put a tag, so offering
   * one would be a control that silently does nothing — the same reason the
   * history link is hidden there.
   */
  enable(on: boolean): void;
  /** Re-reads the server's list, after a run has stored a new tag. */
  refresh(): Promise<void>;
}

/** A picker for a page that has no chip row, so callers need not care. */
const INERT: LocationPicker = {
  current: () => '',
  enable: () => {},
  refresh: () => Promise.resolve(),
};

/**
 * Wires up the chip row, if the page has one.
 *
 * Chips rather than a dropdown, and an inline field rather than a prompt: this
 * is pressed with a thumb while standing in the room being measured, where a
 * modal dialog is a wall between you and the thing you came to do.
 */
export function setUpLocationPicker(): LocationPicker {
  const row = document.getElementById('places');
  const chips = document.getElementById('place-chips');
  const form = document.getElementById('place-new');
  const input = document.getElementById('place-input');
  const cancel = document.getElementById('place-cancel');

  if (
    !row ||
    !chips ||
    !(form instanceof HTMLFormElement) ||
    !(input instanceof HTMLInputElement) ||
    !cancel
  ) {
    return INERT;
  }

  let selected = readStoredLocation();
  let known: string[] = [];
  let asked = false;
  let adding = false;

  const chip = (label: string, value: string, on: boolean): string =>
    `<button class="chip" type="button" data-place="${esc(value)}" data-testid="place-chip"
             title="${esc(label)}" aria-pressed="${on}">${esc(label)}</button>`;

  // Arrow functions rather than declarations throughout: a hoisted `function`
  // could be called before the null checks above, so TypeScript declines to
  // carry their narrowing inside one.
  const paint = (): void => {
    chips.innerHTML = [
      // First and default, so a run is never quietly attributed to wherever
      // the previous one happened to be.
      chip('No location', '', selected === ''),
      ...mergeLocations(known, selected).map((l) => chip(l, l, l === selected)),
      `<button class="chip chip--add" type="button" data-testid="place-add"
               aria-expanded="${adding}">+ Add</button>`,
    ].join('');
  };

  const select = (value: string): void => {
    selected = normaliseLocation(value);
    writeStoredLocation(selected);
    paint();
  };

  const openForm = (): void => {
    adding = true;
    paint();
    form.hidden = false;
    input.value = '';
    input.focus();
  };

  const closeForm = (): void => {
    adding = false;
    form.hidden = true;
    input.value = '';
    paint();
  };

  chips.addEventListener('click', (event) => {
    const button = (event.target as HTMLElement | null)?.closest<HTMLElement>('.chip');
    if (!button) return;

    if (button.classList.contains('chip--add')) {
      if (adding) closeForm();
      else openForm();
      return;
    }
    select(button.dataset.place ?? '');
  });

  form.addEventListener('submit', (event) => {
    event.preventDefault();
    const typed = normaliseLocation(input.value);
    if (typed === '') {
      closeForm();
      return;
    }
    select(matchExisting(mergeLocations(known, selected), typed) ?? typed);
    closeForm();
    // The field it was typed in has just been hidden, so hand focus to the
    // chip it became rather than dropping it on the document.
    chips.querySelector<HTMLElement>('.chip[aria-pressed="true"]')?.focus();
  });

  input.addEventListener('keydown', (event) => {
    if (event.key !== 'Escape') return;
    closeForm();
    chips.querySelector<HTMLElement>('.chip--add')?.focus();
  });

  cancel.addEventListener('click', () => {
    closeForm();
    chips.querySelector<HTMLElement>('.chip--add')?.focus();
  });

  const refresh = async (): Promise<void> => {
    // Never throws: a deployment whose list cannot be read should still let
    // you type a tag.
    known = await fetchLocations();
    paint();
  };

  paint();

  return {
    current: () => selected,
    enable(on: boolean): void {
      row.hidden = !on;
      if (!on || asked) return;
      asked = true;
      void refresh();
    },
    refresh,
  };
}

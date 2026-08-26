/**
 * Light and dark, on every page.
 *
 * Three states rather than two: an explicit choice, remembered; and "no
 * choice", which follows the operating system. That third state is the default
 * and matters more than it looks — a phone that switches to light at sunrise
 * should follow, and a reader who has actually pressed the button should not.
 *
 * The stored value is only ever `light` or `dark`. Clearing it returns to
 * following the system, which is what the button cycles back to.
 */

const KEY = 'speedtest.theme';

export type Choice = 'light' | 'dark' | 'system';

function stored(): Choice {
  try {
    const value = localStorage.getItem(KEY);
    if (value === 'light' || value === 'dark') return value;
  } catch {
    // Private browsing, or storage disabled: follow the system.
  }
  return 'system';
}

/** Applies a choice to the document, and remembers it. */
export function applyTheme(choice: Choice): void {
  if (choice === 'system') {
    delete document.documentElement.dataset.theme;
  } else {
    document.documentElement.dataset.theme = choice;
  }

  try {
    if (choice === 'system') localStorage.removeItem(KEY);
    else localStorage.setItem(KEY, choice);
  } catch {
    // The choice applies to this page and simply will not be remembered.
  }
}

/** What the system currently asks for, when no choice has been made. */
function systemPrefersDark(): boolean {
  return window.matchMedia?.('(prefers-color-scheme: dark)').matches ?? true;
}

/** The label for a button that will switch to the *other* theme. */
function labelFor(choice: Choice): string {
  const dark = choice === 'dark' || (choice === 'system' && systemPrefersDark());
  return dark ? 'Light' : 'Dark';
}

/**
 * Wires up a toggle button, if the page has one.
 *
 * Two states from the button, not three: cycling a reader through "system" as
 * a visible step would be a puzzle, since it looks identical to whichever of
 * light or dark the system happens to be on. System is where you start and
 * where a cleared setting returns you, not somewhere the button stops.
 */
export function setUpThemeToggle(): void {
  const button = document.getElementById('theme-toggle');
  applyTheme(stored());
  if (!(button instanceof HTMLButtonElement)) return;

  const paint = () => {
    const next = labelFor(stored());
    button.textContent = next;
    button.setAttribute('aria-label', `Switch to ${next.toLowerCase()} theme`);
  };
  paint();

  button.addEventListener('click', () => {
    const now = stored();
    const dark = now === 'dark' || (now === 'system' && systemPrefersDark());
    applyTheme(dark ? 'light' : 'dark');
    paint();
  });

  // Following the system means following it while the page is open, not only
  // at load: a laptop that switches at sunset should take the page with it.
  window
    .matchMedia?.('(prefers-color-scheme: dark)')
    .addEventListener?.('change', () => {
      if (stored() === 'system') paint();
    });
}

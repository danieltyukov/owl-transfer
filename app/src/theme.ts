import { useCallback, useEffect, useState } from 'react';

/*
 * Light, dark, or whatever the system says.
 *
 * The choice is a `data-theme` attribute on the root element and a line in
 * local storage, and the token sheet does the rest: `system` is the absence of
 * the attribute, which leaves the media query in charge. Nothing else in the
 * interface reads the theme, because nothing else needs to.
 */

export type Theme = 'system' | 'light' | 'dark';

const KEY = 'owl-theme';

const isTheme = (value: unknown): value is Theme =>
  value === 'system' || value === 'light' || value === 'dark';

/** The stored choice, or the system default when there is none to read. */
export function readTheme(): Theme {
  try {
    const stored = localStorage.getItem(KEY);
    return isTheme(stored) ? stored : 'system';
  } catch {
    // A WebView with storage disabled. The app still themes itself; it just
    // forgets the choice when it closes.
    return 'system';
  }
}

export function applyTheme(theme: Theme): void {
  const root = document.documentElement;
  if (theme === 'system') root.removeAttribute('data-theme');
  else root.setAttribute('data-theme', theme);
  try {
    localStorage.setItem(KEY, theme);
  } catch {
    // See above.
  }
}

export function useTheme(): [Theme, (next: Theme) => void] {
  const [theme, setTheme] = useState<Theme>(readTheme);

  // On mount as well as on change: the stored choice has to reach the document
  // even when nobody touches the control this session.
  useEffect(() => {
    applyTheme(theme);
  }, [theme]);

  return [theme, useCallback((next: Theme) => setTheme(next), [])];
}

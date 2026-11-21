// Browser-language auto-redirect for the docs root pages (EN root `/` ↔ `/zh/`).
// Scope (todo.md #477): root pages only, once per full page load; a manual
// language switch is remembered (localStorage) and never auto-overridden.
// Deep links (e.g. `/cage/schema`) never redirect.
//
// Locale is judged by URL path (locale root), never by page content — the
// English root page legitimately carries CJK snippets, so content sniffing
// would misjudge. `base` is VitePress BASE_URL (`/cage/`).
//
// The pure helpers below are unit-tested by `lang-redirect.test.mjs`
// (node --test, zero dependencies); the browser wiring is a thin layer on top.

export const LANG_STORAGE_KEY = 'cage-docs-lang'

// Strip `base` from `pathname` and return a normalized site-relative path.
export function normalizePath(pathname, base) {
  let p = String(pathname || '')
  const b = base && base !== '/' ? (base.endsWith('/') ? base : base + '/') : '/'
  if (b !== '/') {
    const bare = b.slice(0, -1)
    if (p === bare) p = '/'
    else if (p.startsWith(b)) p = p.slice(b.length - 1)
  }
  if (!p.startsWith('/')) p = '/' + p
  return p
}

// Locale of a path, or null for deep pages / unknown paths.
export function localeOf(pathname, base) {
  const p = normalizePath(pathname, base)
  if (p === '/') return 'en'
  if (p === '/zh/' || p === '/zh') return 'zh'
  return null
}

export function isLocaleRoot(pathname, base) {
  return localeOf(pathname, base) !== null
}

// Locale of *any* page (root or deep) — `/zh/...` is zh, everything else is en.
export function pageLocale(pathname, base) {
  const p = normalizePath(pathname, base)
  if (p === '/zh/' || p === '/zh' || p.startsWith('/zh/')) return 'zh'
  return 'en'
}

// Map `navigator.languages` (or a language string) to a supported locale.
export function browserLocale(languages) {
  const first = (Array.isArray(languages) && languages[0]) || languages || ''
  return String(first).toLowerCase().startsWith('zh') ? 'zh' : 'en'
}

// Absolute path (including base) of a locale root.
export function localeRootPath(locale, base) {
  const b = base && base.endsWith('/') ? base : base + '/'
  return locale === 'zh' ? b + 'zh/' : b
}

// Auto-redirect decision. Returns the target URL path, or null to stay.
// Rules: only locale root pages participate; an explicit stored language
// choice disables auto-redirect entirely; otherwise follow the browser
// language when it differs from the current root's locale.
export function decideRedirect({ pathname, base, stored, languages }) {
  const current = localeOf(pathname, base)
  if (!current) return null
  if (stored) return null
  const want = browserLocale(languages)
  if (want === current) return null
  return localeRootPath(want, base)
}

// Explicit language choice: clicking a link into the *other* locale's root
// (the language dropdown) records that locale. Returns the chosen locale, or
// null if the click is not a cross-locale switch. `from` may be any page; the
// anchor target must be a locale root (only the dropdown links there).
export function manualSwitchTarget(fromPathname, anchorPathname, base) {
  const from = pageLocale(fromPathname, base)
  const to = localeOf(anchorPathname, base)
  if (!to || to === from) return null
  return to
}

// Browser wiring. Runs once per full page load (guarded), so in-app SPA
// navigation — including the nav language dropdown — never re-triggers a
// redirect. Returns false when running outside a browser (SSR/SSG build).
export function installLangRedirect() {
  if (typeof window === 'undefined') return false
  if (installLangRedirect._ran) return false
  installLangRedirect._ran = true

  const base =
    (typeof import.meta !== 'undefined' && import.meta.env && import.meta.env.BASE_URL) || '/'

  const run = () => {
    let stored = null
    try {
      stored = window.localStorage.getItem(LANG_STORAGE_KEY)
    } catch {
      stored = null
    }
    const target = decideRedirect({
      pathname: window.location.pathname,
      base,
      stored,
      languages: window.navigator.languages || [window.navigator.language],
    })
    if (target && target !== window.location.pathname) {
      // replace() so the redirected root is not left in history.
      window.location.replace(target)
    }
  }
  run()

  // Record an explicit switch (language dropdown / any cross-root link).
  document.addEventListener(
    'click',
    (e) => {
      const a = e.target && e.target.closest ? e.target.closest('a') : null
      if (!a) return
      const to = manualSwitchTarget(window.location.pathname, a.pathname, base)
      if (to) {
        try {
          window.localStorage.setItem(LANG_STORAGE_KEY, to)
        } catch {
          // storage unavailable — auto-redirect just stays on next load
        }
      }
    },
    true
  )
  return true
}

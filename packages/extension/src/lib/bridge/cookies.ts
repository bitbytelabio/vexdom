import { browser, type Browser } from 'wxt/browser';
import { z } from 'zod';
import { cookies } from '@/lib/actions/page/cookies';
import { failure, success, type ActionResult } from '@/lib/actions/protocol';

type Cookie = Browser.cookies.Cookie;
type SameSite = 'strict' | 'lax' | 'no_restriction';

const describe = (cookie: Cookie) => ({
  name: cookie.name,
  value: cookie.value,
  domain: cookie.domain,
  path: cookie.path,
  secure: cookie.secure,
  httpOnly: cookie.httpOnly,
  sameSite: cookie.sameSite,
  expires: cookie.expirationDate,
});

/** `page.cookies` for `url`, the tab's address when the input names none. */
export async function cookiesFor(input: unknown, tabUrl?: string): Promise<ActionResult> {
  const parsed = cookies.input.safeParse(input ?? {});
  if (!parsed.success) return failure('INVALID_INPUT', z.prettifyError(parsed.error));
  const { op, name, value, domain, path, secure, httpOnly, sameSite, expires } = parsed.data;
  const url = parsed.data.url ?? tabUrl;
  if (!url || !/^https?:/i.test(url)) return failure('INVALID_INPUT', 'page.cookies needs an http(s) "url", or a tab on a web page.');
  try {
    if (op === 'get') {
      const found = await browser.cookies.getAll({ url, ...(name ? { name } : {}) });
      return success({ cookies: found.map(describe) });
    }
    if (op === 'set') {
      if (!name || value === undefined) return failure('INVALID_INPUT', '"set" needs a "name" and a "value".');
      const set = await browser.cookies.set({
        url,
        name,
        value,
        domain,
        path,
        secure,
        httpOnly,
        sameSite: sameSite as SameSite | undefined as Browser.cookies.SameSiteStatus | undefined,
        expirationDate: expires,
      });
      if (!set) return failure('ACTION_FAILED', `The browser refused the cookie "${name}".`);
      return success({ set: describe(set) });
    }
    const found = await browser.cookies.getAll({ url, ...(name ? { name } : {}) });
    for (const cookie of found) {
      const scheme = cookie.secure ? 'https' : 'http';
      await browser.cookies.remove({ url: `${scheme}://${cookie.domain.replace(/^\./, '')}${cookie.path}`, name: cookie.name });
    }
    return success({ removed: found.length });
  } catch (error) {
    return failure('ACTION_FAILED', error instanceof Error ? error.message : String(error));
  }
}

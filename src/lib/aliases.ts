// User-defined slash commands: `/ns identify x` → `/msg NickServ identify x`.

export interface AliasVars {
  nick: string;
  channel: string;
  network: string;
}

/** Split a body into its `;`-separated commands. `\;` is a literal semicolon. */
export function splitCommands(body: string): string[] {
  return body
    .split(/(?<!\\);/)
    .map((c) => c.replace(/\\;/g, ';').trim())
    .filter(Boolean);
}

/** Substitute `$1`…`$9`, `$2-` (from the 2nd on), `$*`, `$nick`, `$channel` and `$network`. */
export function expandCommand(cmd: string, args: string[], vars: AliasVars): string {
  return cmd
    .replace(/\$(?:(\*)|([1-9])(-?)|(nick|channel|network)(?![A-Za-z]))/g, (_, star, n, dash, name) => {
      if (star) return args.join(' ');
      if (n) return dash ? args.slice(+n - 1).join(' ') : (args[+n - 1] ?? '');
      return vars[name as keyof AliasVars];
    })
    .trimEnd();
}

/** `/J ` → `j`. */
export const aliasName = (raw: string) => raw.trim().replace(/^\/+/, '').toLowerCase();

/** An error for a name or body that can't work, else null. */
export function aliasError(name: string, body: string): string | null {
  if (!name) return 'Give the alias a name';
  if (!/^[a-z0-9_-]+$/.test(name)) return 'Names use letters, digits, - and _ only';
  if (!splitCommands(body).length) return 'Add at least one command';
  return null;
}

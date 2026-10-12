// The small subset of Markdown the AI writes in chat: paragraphs, headings,
// bullet and numbered lists (one level of nesting), tables, **bold**, *italic*
// and `code`. Rendered as React elements, never as HTML, so nothing it writes
// can run in the page.

import { Fragment, type ReactNode } from "react";

function inline(text: string): ReactNode[] {
  const out: ReactNode[] = [];
  const pattern = /(\*\*[^*]+\*\*|`[^`]+`|\*[^*\s][^*]*\*)/g;
  let last = 0;
  let m: RegExpExecArray | null;
  while ((m = pattern.exec(text))) {
    if (m.index > last) out.push(text.slice(last, m.index));
    const t = m[0];
    if (t.startsWith("**")) out.push(<strong key={m.index}>{t.slice(2, -2)}</strong>);
    else if (t.startsWith("`")) out.push(<code key={m.index}>{t.slice(1, -1)}</code>);
    else out.push(<em key={m.index}>{t.slice(1, -1)}</em>);
    last = m.index + t.length;
  }
  if (last < text.length) out.push(text.slice(last));
  return out;
}

const BULLET = /^\s*[-•*]\s+/;
const NUMBERED = /^\s*\d+[.)]\s+/;
const ROW = /^\s*\|.*\|\s*$/;
const RULE = /^\s*\|?(\s*:?-{2,}:?\s*\|)+\s*(:?-{2,}:?\s*)?\|?\s*$/;
const HEADING = /^\s*#{1,6}\s+/;

const isList = (l: string) => BULLET.test(l) || NUMBERED.test(l);
const indent = (l: string) => l.match(/^\s*/)![0].length;
const cells = (l: string) => l.trim().replace(/^\|/, "").replace(/\|$/, "").split("|").map((c) => c.trim());

/** A list from line `i`; lines indented past its first item nest under the item before them. */
function list(lines: string[], i: number): [ReactNode, number] {
  const ordered = NUMBERED.test(lines[i]);
  const base = indent(lines[i]);
  const items: { text: string; sub: ReactNode | null }[] = [];
  while (i < lines.length && isList(lines[i]) && indent(lines[i]) >= base) {
    if (indent(lines[i]) > base && items.length) {
      const [sub, next] = list(lines, i);
      items[items.length - 1].sub = sub;
      i = next;
      continue;
    }
    items.push({ text: lines[i].replace(BULLET.test(lines[i]) ? BULLET : NUMBERED, ""), sub: null });
    i++;
  }
  const List = ordered ? "ol" : "ul";
  return [
    <List key={i}>
      {items.map((t, k) => (
        <li key={k}>
          {inline(t.text)}
          {t.sub}
        </li>
      ))}
    </List>,
    i,
  ];
}

export function Markdown({ text }: { text: string }) {
  const blocks: ReactNode[] = [];
  const lines = text.split("\n");
  let i = 0;
  while (i < lines.length) {
    const line = lines[i];
    if (!line.trim()) {
      i++;
      continue;
    }
    if (HEADING.test(line)) {
      blocks.push(<h4 key={i}>{inline(line.replace(HEADING, ""))}</h4>);
      i++;
      continue;
    }
    if (isList(line)) {
      const [node, next] = list(lines, i);
      blocks.push(node);
      i = next;
      continue;
    }
    if (ROW.test(line) && i + 1 < lines.length && RULE.test(lines[i + 1])) {
      const head = cells(line);
      const align = cells(lines[i + 1]).map((c) => (c.endsWith(":") ? (c.startsWith(":") ? "center" : "right") : "left") as "left" | "right" | "center");
      i += 2;
      const rows: string[][] = [];
      while (i < lines.length && ROW.test(lines[i])) rows.push(cells(lines[i++]));
      blocks.push(
        <div className="md-table" key={i}>
          <table>
            <thead>
              <tr>
                {head.map((c, k) => (
                  <th key={k} style={{ textAlign: align[k] }}>
                    {inline(c)}
                  </th>
                ))}
              </tr>
            </thead>
            <tbody>
              {rows.map((r, n) => (
                <tr key={n}>
                  {head.map((_, k) => (
                    <td key={k} style={{ textAlign: align[k] }}>
                      {inline(r[k] ?? "")}
                    </td>
                  ))}
                </tr>
              ))}
            </tbody>
          </table>
        </div>,
      );
      continue;
    }
    const para: string[] = [];
    while (i < lines.length && lines[i].trim() && !isList(lines[i]) && !HEADING.test(lines[i]) && !(ROW.test(lines[i]) && RULE.test(lines[i + 1] ?? ""))) para.push(lines[i++]);
    blocks.push(
      <p key={i}>
        {para.map((t, k) => (
          <Fragment key={k}>
            {k > 0 && <br />}
            {inline(t)}
          </Fragment>
        ))}
      </p>,
    );
  }
  return <>{blocks}</>;
}

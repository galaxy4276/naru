/** One proven problem in a `naru proof` report. */
export interface Finding {
  title: string;
  foundBy: string | null;
  command: string | null;
  output: string | null;
  /** How the failure breaks the change's intent. */
  note: string;
}

export interface Review {
  findings: Finding[];
  /** One line per adversary, as written. */
  dropped: string[];
  droppedCount: number;
}

/**
 * A `naru proof` report: `## Findings` with one `###` per finding, its first code block the
 * command and its second the output, and an optional `## Dropped` list. `null` without
 * `## Findings`.
 */
export function parseReview(markdown: string): Review | null {
  let section: 'other' | 'findings' | 'dropped' = 'other';
  let sawFindings = false;
  const findings: Finding[] = [];
  const dropped: string[] = [];
  let fence: string[] | null = null;
  let blocks = 0;
  for (const line of markdown.split('\n')) {
    if (fence) {
      if (line.trimStart().startsWith('```')) {
        const last = findings.at(-1);
        if (last && blocks === 0) last.command = fence.join('\n');
        else if (last && blocks === 1) last.output = fence.join('\n');
        blocks += 1;
        fence = null;
      } else {
        fence.push(line);
      }
      continue;
    }
    const trimmed = line.trim();
    if (trimmed.startsWith('## ')) {
      const heading = trimmed.slice(3).toLowerCase();
      section = heading.includes('finding') ? 'findings' : heading.includes('dropped') ? 'dropped' : 'other';
      if (section === 'findings') sawFindings = true;
      continue;
    }
    if (section === 'findings') {
      const last = findings.at(-1);
      if (trimmed.startsWith('### ')) {
        findings.push({ title: trimmed.slice(4).trim(), foundBy: null, command: null, output: null, note: '' });
        blocks = 0;
      } else if (trimmed.startsWith('```')) {
        fence = [];
      } else if (last) {
        const who = trimmed.match(/^(?:\*\*Found by:\*\*|Found by:)(.*)$/);
        if (who) last.foundBy = who[1].trim();
        else if (trimmed) last.note += (last.note ? ' ' : '') + trimmed;
      }
    } else if (section === 'dropped') {
      const item = trimmed.match(/^[-*] (.*)$/);
      if (item) dropped.push(item[1].trim());
    }
  }
  if (!sawFindings) return null;
  // ponytail: the first number after the line's last `:` (so `adversary-2` is skipped), or on the
  // whole line without one; a line without one adds nothing.
  const droppedCount = dropped.reduce((sum, line) => {
    const tail = line.includes(':') ? line.slice(line.lastIndexOf(':') + 1) : line;
    const digits = tail.match(/\d+/);
    return sum + (digits ? Number(digits[0]) : 0);
  }, 0);
  return { findings, dropped, droppedCount };
}

export interface Decision {
  at: string;
  phase: string;
  decision: string;
  why: string;
  evidence: string;
  result: string;
}

export function reverted(decision: Decision): boolean {
  const result = decision.result.toLowerCase();
  return ['되돌', 'revert', 'rolled back'].some((word) => result.includes(word));
}

/** A show-me-your-work log's rows, oldest first, and how many lines lacked its six columns. */
export function parseDecisions(tsv: string): { rows: Decision[]; broken: number } {
  const rows: Decision[] = [];
  let broken = 0;
  tsv.split('\n').forEach((line, index) => {
    if (!line.trim() || (index === 0 && line.startsWith('ts\t'))) return;
    const cells = line.split('\t');
    if (cells.length !== 6) {
      broken += 1;
      return;
    }
    const [at, phase, decision, why, evidence, result] = cells;
    rows.push({ at, phase, decision, why, evidence, result });
  });
  return { rows, broken };
}

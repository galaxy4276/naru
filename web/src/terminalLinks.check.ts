import { findLinks } from './terminalLinks.ts';

const expect = (line: string, want: string[]) => {
  const got = texts(line).join('|');
  if (got !== want.join('|')) throw new Error(`${line}: got ${got}`);
};
const texts = (line: string) => findLinks(line).map((link) => `${link.url ? 'url' : 'path'}:${link.text}`);
expect('see https://github.com/x/y/pull/12. done', ['url:https://github.com/x/y/pull/12']);
expect('error at src/ui/App.tsx:418:73: bad', ['path:src/ui/App.tsx:418:73']);
expect('wrote ~/notes/a.md and /tmp/x/y.log', ['path:~/notes/a.md', 'path:/tmp/x/y.log']);
expect('편집: CLAUDE.md 끝', ['path:CLAUDE.md']);
expect('just words and 3.14 and e.g. here', []);
expect('open http://localhost:5173/', ['url:http://localhost:5173/']);
console.log('links ok');

import { anchorAt, blockOf, locate } from './anchor.ts';

const same = (actual: unknown, expected: unknown, message = '') => {
  if (JSON.stringify(actual) !== JSON.stringify(expected)) throw new Error(`${message}: ${JSON.stringify(actual)} ≠ ${JSON.stringify(expected)}`);
};

const text = '좋은 질문. 과거의 일을 물어라. 그리고 다시, 과거의 일을 물어라. 끝.';
const second = text.lastIndexOf('과거의 일');
const anchor = anchorAt(text, second, second + '과거의 일'.length);
same(locate(text, anchor), [second, second + '과거의 일'.length], 'the second of two equal passages');

const retranslated = '새 문장이 앞에 붙었다. ' + text;
const moved = retranslated.lastIndexOf('과거의 일');
same(locate(retranslated, anchor), [moved, moved + '과거의 일'.length], 'found again after the text moved');
same(locate('전혀 다른 번역', anchor), null);
same(locate(text, { quote: '', prefix: '', suffix: '' }), null);

const blocks = ['첫 문단', '좋은 질문은 세 가지를 갖춘다. 과거에 있었던 일이다.', '마지막'];
same(blockOf(blocks, { quote: '세 가지를 갖춘다. 과거에 있었던 일이다.', block: 0 }), 1);
same(blockOf(blocks, { quote: '사라진 문장', block: 7 }), 2, 'falls back to its block, kept on the page');
same(blockOf([], { quote: '무엇이든', block: 0 }), -1);

console.log('anchor ok');

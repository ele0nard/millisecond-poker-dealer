import { readFile } from 'node:fs/promises';
import { verifyShuffle } from './verify-shuffle.mjs';

async function main() {
  const [handPath, manifestPath] = process.argv.slice(2);
  if (!handPath || !manifestPath) {
    throw new Error('Usage: node proof/check-hand.mjs HAND.json MANIFEST.txt');
  }

  const payload = JSON.parse(await readFile(handPath, 'utf8'));
  const [version, ...entries] = (await readFile(manifestPath, 'utf8')).trimEnd().split('\n');
  if (version !== 'ms-poker-timestamp-v1') throw new Error('Unknown timestamp manifest format.');
  const roots = entries
    .map(line => line.split(' '))
    .filter(([handId]) => handId === payload.proof.hand_id);
  if (roots.length !== 1 || roots[0].length !== 2 || !/^[0-9a-f]{64}$/.test(roots[0][1])) {
    throw new Error('The manifest must contain exactly one valid root for this hand.');
  }
  if (!await verifyShuffle(payload, roots[0][1])) {
    throw new Error('The card openings did not verify against the manifest commitment.');
  }
  const { board, showdown } = payload.proof;
  const count = board.length + showdown.length * 2;
  console.log(count
    ? `${count} revealed cards match the manifest commitment.`
    : 'The root matches the manifest. This hand has no public card openings.');
}

main().catch(error => {
  console.error(error.message);
  process.exitCode = 1;
});

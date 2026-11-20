const version = 'ms-poker-deck-v1';
const uuid = /^[0-9a-f]{8}(?:-[0-9a-f]{4}){3}-[0-9a-f]{12}$/i;
const hashPattern = /^[0-9a-f]{64}$/i;
const encoder = new TextEncoder();

function bytes(hex) {
  return Uint8Array.from(hex.match(/../g), pair => parseInt(pair, 16));
}

async function sha256(...parts) {
  const data = new Uint8Array(parts.reduce((sum, part) => sum + part.length, 0));
  let offset = 0;
  for (const part of parts) {
    data.set(part, offset);
    offset += part.length;
  }
  return new Uint8Array(await crypto.subtle.digest('SHA-256', data));
}

function hex(data) {
  return Array.from(data, byte => byte.toString(16).padStart(2, '0')).join('');
}

export async function verifyShuffle(payload, expectedRoot) {
  const {proof} = payload;
  if (payload.version !== version || !uuid.test(proof.hand_id) ||
      !Number.isInteger(proof.seats) || proof.seats < 2 || proof.seats > 6 ||
      !Number.isInteger(proof.button) || proof.button < 0 || proof.button >= proof.seats ||
      !hashPattern.test(proof.root) || proof.root !== expectedRoot.toLowerCase()) return false;
  if (proof.state !== 'complete' || !Array.isArray(proof.board) ||
      ![0, 3, 4, 5].includes(proof.board.length) || !Array.isArray(proof.showdown)) return false;
  const positions = [1, 2, 3, 5, 7].map(offset => 2 * proof.seats + offset);
  const checks = proof.board.map((card, index) => [card, positions[index]]);
  const shown = new Set();
  for (const {seat, cards} of proof.showdown) {
    if (!Number.isInteger(seat) || seat < 0 || seat >= proof.seats || shown.has(seat) ||
        !Array.isArray(cards) || cards.length !== 2 || proof.board.length !== 5) return false;
    shown.add(seat);
    const start = (seat + proof.seats - proof.button - 1) % proof.seats;
    checks.push([cards[0], start], [cards[1], start + proof.seats]);
  }
  const visibleCards = new Set();
  for (const [opening, position] of checks) {
    if (opening.position !== position || !/^[2-9TJQKA][cdhs]$/.test(opening.card) ||
        visibleCards.has(opening.card) || !hashPattern.test(opening.salt) ||
        !Array.isArray(opening.siblings) || opening.siblings.length !== 6 ||
        !opening.siblings.every(sibling => hashPattern.test(sibling))) return false;
    visibleCards.add(opening.card);
    let node = await sha256(Uint8Array.of(0), encoder.encode(version),
      bytes(proof.hand_id.replaceAll('-', '')), Uint8Array.of(proof.button, proof.seats, position),
      encoder.encode(opening.card), bytes(opening.salt));
    let index = position;
    for (const sibling of opening.siblings) {
      const other = bytes(sibling);
      node = await (index % 2 ? sha256(Uint8Array.of(1), other, node) : sha256(Uint8Array.of(1), node, other));
      index >>= 1;
    }
    if (hex(node) !== expectedRoot.toLowerCase()) return false;
  }
  return true;
}

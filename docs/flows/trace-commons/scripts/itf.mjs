// Match the pinned statechart view's JSON projection, including map keys and
// one-based sequences. Model integers are bounded; reject lossy conversion.
export function decode(v) {
  if (Array.isArray(v)) return v.map(decode);
  if (v && typeof v === 'object') {
    if ('#bigint' in v) {
      const integer = Number(v['#bigint']);
      if (!Number.isSafeInteger(integer)) throw Error('ITF integer exceeds safe integer range');
      return integer;
    }
    if ('#set' in v) return v['#set'].map(decode);
    if ('#tup' in v) return Object.fromEntries(v['#tup'].map((item, i) => [String(i + 1), decode(item)]));
    if ('#map' in v) return Object.fromEntries(v['#map'].map(([k, value]) => {
      const key = decode(k);
      return [typeof key === 'object' ? JSON.stringify(key) : String(key), decode(value)];
    }));
    if (typeof v.tag === 'string' && Object.hasOwn(v, 'value') && Object.keys(v).length === 2) {
      const unit = v.value && typeof v.value === 'object' && Array.isArray(v.value['#tup']) && v.value['#tup'].length === 0;
      return {[v.tag]: unit ? {tag: 'UNIT'} : decode(v.value)};
    }
    return Object.fromEntries(Object.entries(v).map(([k, value]) => [k, decode(value)]));
  }
  return v;
}

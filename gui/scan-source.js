const bytes = value => new TextEncoder().encode(value).length;
export function scanPublicationSource(owner) { return `scan/${bytes(owner)}:${owner}`; }
export function scanPublicationOwner(source) {
  if (typeof source !== 'string' || !source.startsWith('scan/')) return null;
  const colon=source.indexOf(':',5); if(colon<0)return null;
  const owner=source.slice(colon+1);
  return owner && bytes(owner)<=128 && !/\p{Cc}/u.test(owner)
    && scanPublicationSource(owner)===source ? owner : null;
}

import { avatarColour, initials } from '../features/mailbox/selectors';

export function Avatar({
  name,
  address,
  size = 36,
  photo = null,
}: {
  name: string;
  address: string;
  size?: number;
  /** A contact's picture (`data:` URL); initials when missing. */
  photo?: string | null;
}) {
  if (photo) {
    return <img className="avatar avatar--photo" src={photo} alt="" aria-hidden="true" width={size} height={size} />;
  }
  return (
    <span
      className="avatar"
      aria-hidden="true"
      style={{ width: size, height: size, fontSize: Math.round(size * 0.36), background: avatarColour(address) }}
    >
      {initials(name || address)}
    </span>
  );
}

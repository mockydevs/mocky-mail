import { avatarColour, initials } from '../features/mailbox/selectors';

export function Avatar({ name, address, size = 36 }: { name: string; address: string; size?: number }) {
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

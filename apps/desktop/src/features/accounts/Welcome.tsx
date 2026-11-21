import type { Account } from '../../api/types';
import { Icon, Logo, type IconName } from '../../ui/Icon';
import { AddAccountForm } from './AddAccountForm';

const POINTS: readonly { icon: IconName; title: string; text: string }[] = [
  { icon: 'device', title: 'Your mail lives on this device', text: 'Read and write even without a connection.' },
  { icon: 'lock', title: 'Passwords stay in the system password store', text: 'Never saved in plain text.' },
  { icon: 'mail', title: 'Works with any mailbox', text: 'Gmail, Outlook, or your own mail server.' },
];

/** First run: a brand panel beside the add-account card (stacked on a phone). */
export function Welcome({ onAdded }: { onAdded: (account: Account) => void }) {
  return (
    <main className="welcome">
      <section className="welcome__intro">
        <div className="brand">
          <Logo />
          <span className="brand__name">Mocky Mail</span>
        </div>
        <div className="welcome__body">
          <h1 className="welcome__title">Welcome to Mocky Mail</h1>
          <p className="welcome__text">Add your first email account to get started.</p>
          <ul className="welcome__points">
            {POINTS.map((point) => (
              <li key={point.icon}>
                <Icon name={point.icon} size={20} />
                <div>
                  <strong>{point.title}</strong>
                  <span>{point.text}</span>
                </div>
              </li>
            ))}
          </ul>
        </div>
        <p className="welcome__foot">Private, open-source email · mailer.co.ke</p>
        <span className="welcome__mark">
          <Logo />
        </span>
      </section>
      <div className="welcome__main">
        <AddAccountForm onAdded={onAdded} />
      </div>
    </main>
  );
}

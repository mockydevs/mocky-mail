import type { Account } from '../../api/types';
import { Logo } from '../../ui/Icon';
import { AddAccountForm } from './AddAccountForm';

export function Welcome({ onAdded }: { onAdded: (account: Account) => void }) {
  return (
    <main className="welcome">
      <div className="welcome__intro">
        <div className="brand">
          <Logo />
          <span className="brand__name">Mocky Mail</span>
        </div>
        <h1 className="welcome__title">Welcome to Mocky Mail</h1>
        <p className="welcome__text">
          Add your first email account to get started. Your mail is kept on this computer, so you can read and write
          even without a connection.
        </p>
      </div>
      <AddAccountForm onAdded={onAdded} />
    </main>
  );
}

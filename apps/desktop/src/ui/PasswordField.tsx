import { useState, type Ref } from 'react';

import { Icon } from './Icon';

interface Props {
  id: string;
  inputRef: Ref<HTMLInputElement>;
  /** `off`: a password used once (a key file's), which nothing should offer to save. */
  autoComplete: 'current-password' | 'new-password' | 'off';
  describedBy?: string;
  invalid?: boolean;
}

/**
 * A password input with a show/hide button, so people can check a long app
 * password before saving it. The value stays in the uncontrolled input; the
 * button only switches the input type.
 */
export function PasswordField({ id, inputRef, autoComplete, describedBy, invalid }: Props) {
  const [visible, setVisible] = useState(false);
  return (
    <div className="password-field">
      <input
        id={id}
        ref={inputRef}
        className="field"
        type={visible ? 'text' : 'password'}
        autoComplete={autoComplete}
        autoCapitalize="off"
        autoCorrect="off"
        spellCheck={false}
        aria-describedby={describedBy}
        aria-invalid={invalid ? true : undefined}
      />
      <button
        type="button"
        className="password-field__toggle"
        aria-label="Show password"
        aria-pressed={visible}
        aria-controls={id}
        title={visible ? 'Hide password' : 'Show password'}
        onClick={() => {
          setVisible((on) => !on);
        }}
      >
        <Icon name={visible ? 'eyeOff' : 'eye'} />
      </button>
    </div>
  );
}

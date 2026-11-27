import { errorTitle } from '../../api/errors';
import type { AppError } from '../../api/types';

export interface FileErrorCopy {
  title: string;
  text: string;
}

/**
 * An import or export that failed, in plain words: no connection, the mail
 * server said no, or the file itself could not be used. The backend's own
 * message follows as the detail.
 */
export function fileErrorCopy(error: AppError, workOffline = false): FileErrorCopy {
  switch (error.kind) {
    case 'connect':
      return workOffline
        ? { title: "You're working offline", text: 'Turn off Work Offline, then try again. Nothing was changed.' }
        : {
            title: "Couldn't reach the mail server",
            text: `Check that you're online, then try again. ${error.message}`,
          };
    case 'auth':
      return {
        title: 'The mail server refused',
        text: `It did not accept your sign-in. Sign in again in Settings › Accounts, then try again. ${error.message}`,
      };
    case 'tls':
      return { title: 'The mail server refused', text: `A secure connection could not be made. ${error.message}` };
    case 'invalid':
    case 'not_found':
      return { title: "This file couldn't be used", text: error.message };
    case 'too_large':
      return { title: 'Too large for the mail server', text: error.message };
    case 'storage':
      return { title: "Couldn't save on this device", text: `There may not be enough space. ${error.message}` };
    case 'config':
    case 'keyring':
    case 'internal':
      return { title: errorTitle(error.kind), text: error.message };
  }
}

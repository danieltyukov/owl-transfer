import type { PairingInfo } from '../../backend/types.js';
import './PairingCard.css';

export interface PairingCardProps {
  pairing: PairingInfo;
  onRespond: (accept: boolean) => void;
}

/*
 * The six digits, which are the whole of the security model a person can see.
 *
 * Both devices compute the same code from the two certificates and a nonce.
 * Matching codes mean the two are talking to each other and not to something
 * in between, so the code is the largest thing on the screen and the only
 * place the mono runs at that size. Its tracking groups the digits, because
 * the job is to be read off one screen and checked against another.
 */
export function PairingCard({ pairing, onRespond }: PairingCardProps) {
  const incoming = pairing.direction === 'incoming';
  return (
    <section className="pairing" aria-label="Pairing">
      <p className="pairing-lead">
        {incoming ? `${pairing.name} wants to pair` : `Pairing with ${pairing.name}`}
      </p>

      <p className="pairing-code mono">{pairing.code}</p>

      <p className="pairing-note">
        {incoming
          ? 'Accept if the other device shows the same code.'
          : 'Check the code matches, then accept on the other device.'}
      </p>

      <div className="pairing-actions">
        {incoming ? (
          <>
            <button type="button" className="button" onClick={() => onRespond(false)}>
              Decline
            </button>
            <button
              type="button"
              className="button button-primary button-tall"
              onClick={() => onRespond(true)}
            >
              Accept
            </button>
          </>
        ) : (
          <button type="button" className="button" onClick={() => onRespond(false)}>
            Cancel
          </button>
        )}
      </div>
    </section>
  );
}

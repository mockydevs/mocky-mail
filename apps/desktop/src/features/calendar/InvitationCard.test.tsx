import { screen, within } from "@testing-library/react";

import { at, MOCK_MESSAGES } from "../../api/mock/mockData";
import type { MockMessage } from "../../api/mock/mockMailApi";
import type { InvitationView } from "../../api/types";
import { renderApp, rows } from "../../test/renderApp";

const first = MOCK_MESSAGES[1];
if (!first) throw new Error("sample messages missing");
const template: MockMessage = first;

const EVENT: InvitationView["event"] = {
  uid: "abc123@google.com",
  recurrenceId: null,
  title: "Site visit: Kilimani",
  location: "Kilimani, Nairobi",
  notes: "",
  start: at("2026-11-24T11:00:00Z"),
  end: at("2026-11-24T12:00:00Z"),
  allDay: false,
  repeat: "none",
  recurring: false,
  organizer: {
    email: "peter@client.example",
    name: "Peter Mwangi",
    status: "accepted",
  },
  attendees: [
    { email: "peter@client.example", name: "Peter Mwangi", status: "accepted" },
    { email: "you@work.example", name: null, status: "needs-action" },
  ],
  tentative: false,
};

function invite(
  partial: Partial<InvitationView>,
  subject: string,
): MockMessage {
  return {
    ...template,
    id: "inv1",
    subject,
    from: { name: "Peter Mwangi", address: "peter@client.example" },
    preview: "Invitation from Google Calendar",
    unread: true,
    date: at("2026-11-13T07:00:00Z"),
    invitation: {
      method: "REQUEST",
      event: EVENT,
      sequence: 0,
      myStatus: "needs-action",
      replier: null,
      calendarEventId: null,
      savedStatus: null,
      replyPending: false,
      hasCalendar: true,
      ...partial,
    },
  };
}

async function openInvite(message: MockMessage) {
  const ctx = renderApp({ messages: [...MOCK_MESSAGES, message] });
  await ctx.user.click(
    (await rows()).getByRole("button", { name: new RegExp(message.subject) }),
  );
  const card = within(await screen.findByRole("region", { name: EVENT.title }));
  return { ...ctx, card };
}

describe("invitations in mail", () => {
  it("shows an invitation and answers it, emailing the organiser", async () => {
    const { user, api, card } = await openInvite(
      invite({}, "Invitation: Site visit"),
    );
    expect(card.getByText("Invitation")).toBeInTheDocument();
    expect(card.getByText("Kilimani, Nairobi")).toBeInTheDocument();
    expect(
      card.getByText(/From Peter Mwangi, with 1 others/),
    ).toBeInTheDocument();
    expect(
      card.getByRole("checkbox", { name: "Send my answer to Peter Mwangi" }),
    ).toBeChecked();

    await user.click(card.getByRole("button", { name: "Accept" }));
    expect(
      await card.findByText(
        /Accepted\. Peter Mwangi has been told\. It is in your calendar\./,
      ),
    ).toBeInTheDocument();
    // The chosen answer is marked and can't be sent twice.
    const accepted = card.getByRole("button", { name: "Accepted" });
    expect(accepted).toHaveAttribute("aria-pressed", "true");
    expect(accepted).toBeDisabled();
    expect(card.getByRole("button", { name: "Maybe" })).toBeEnabled();
    const outbox = await api.listOutbox();
    expect(
      outbox.some((o) => o.subject === "Accepted: Site visit: Kilimani"),
    ).toBe(true);
  });

  it("can answer without emailing the organiser", async () => {
    const { user, api, card } = await openInvite(
      invite({}, "Invitation: Site visit"),
    );
    const before = (await api.listOutbox()).length;
    await user.click(
      card.getByRole("checkbox", { name: "Send my answer to Peter Mwangi" }),
    );
    await user.click(card.getByRole("button", { name: "Decline" }));
    expect(
      await card.findByText(/Declined\. Nobody was told\./),
    ).toBeInTheDocument();
    expect(await api.listOutbox()).toHaveLength(before);
  });

  it("offers to remove a cancelled event from the calendar", async () => {
    const { user, card } = await openInvite(
      invite(
        { method: "CANCEL", calendarEventId: "e9", savedStatus: "accepted" },
        "Cancelled: Site visit",
      ),
    );
    expect(card.getByText("Cancelled event")).toBeInTheDocument();
    expect(
      card.queryByRole("button", { name: "Accept" }),
    ).not.toBeInTheDocument();
    await user.click(
      card.getByRole("button", { name: "Remove from my calendar" }),
    );
    expect(
      await card.findByText(
        "The organiser cancelled this. It isn't in your calendar.",
      ),
    ).toBeInTheDocument();
  });

  it("records someone's answer to your invitation", async () => {
    const { user, card } = await openInvite(
      invite(
        {
          method: "REPLY",
          replier: {
            email: "peter@client.example",
            name: "Peter Mwangi",
            status: "declined",
          },
          calendarEventId: "e9",
          replyPending: true,
        },
        "Declined: Site visit",
      ),
    );
    expect(
      card.getByText("Peter Mwangi declined your invitation."),
    ).toBeInTheDocument();
    await user.click(card.getByRole("button", { name: "Update my calendar" }));
    expect(
      card.queryByRole("button", { name: "Update my calendar" }),
    ).not.toBeInTheDocument();
  });
});

import { screen, waitFor, within } from "@testing-library/react";

import { at } from "../../api/mock/mockData";
import { renderApp, rows } from "../../test/renderApp";

// "Today" for the calendar: Fri 13 Nov 2026, like the sample mail.
beforeEach(() => {
  vi.useFakeTimers({ toFake: ["Date"] });
  vi.setSystemTime(new Date("2026-11-13T09:00:00Z"));
});

afterEach(() => {
  vi.useRealTimers();
});

async function openCalendar(options: Parameters<typeof renderApp>[0] = {}) {
  const ctx = renderApp(options);
  await rows();
  await ctx.user.click(screen.getByRole("button", { name: "Calendar" }));
  const region = within(
    await screen.findByRole("region", { name: "November 2026" }),
  );
  await region.findAllByRole("button", { name: /Team stand-up/ });
  return { ...ctx, region };
}

/** Just what `day` needs from a `within(...)` result. */
interface Region {
  getByRole: (role: "gridcell", options: { name: RegExp }) => HTMLElement;
}
const day = (region: Region, name: RegExp) =>
  within(region.getByRole("gridcell", { name }));

describe("calendar", () => {
  it("shows the month and an event with its people", async () => {
    const { user, region } = await openCalendar();
    const friday = day(region, /^Fri, Nov 13, 2 events$/);
    await user.click(
      friday.getByRole("button", { name: /Design review with Acme/ }),
    );

    const card = within(
      region.getByRole("article", { name: "Design review with Acme" }),
    );
    expect(card.getByText("Westlands office, room 2")).toBeInTheDocument();
    expect(card.getByText("Bring the printed mock-ups.")).toBeInTheDocument();
    expect(card.getByText("Amina Yusuf")).toBeInTheDocument();
    expect(card.getByText("No answer yet")).toBeInTheDocument();

    // A day with nothing on it, and the all-day launch across two days.
    expect(
      region.getByRole("gridcell", { name: /^Tue, Nov 3, no events$/ }),
    ).toBeInTheDocument();
    expect(
      region.getByRole("gridcell", { name: /^Wed, Nov 18, 2 events$/ }),
    ).toBeInTheDocument();
    expect(
      region.getByRole("gridcell", { name: /^Thu, Nov 19, 2 events$/ }),
    ).toBeInTheDocument();

    await user.click(card.getByRole("button", { name: "Email everyone" }));
    const compose = await screen.findByRole("form", { name: "New message" });
    expect(
      within(
        within(compose).getByRole("list", { name: "To recipients" }),
      ).getAllByRole("listitem"),
    ).toHaveLength(3);
  });

  it("adds an event and moves between months", async () => {
    const { user, api, region } = await openCalendar();
    await user.click(region.getByRole("gridcell", { name: /^Thu, Nov 26/ }));
    await user.click(region.getByRole("button", { name: "New event" }));
    const form = within(region.getByRole("form", { name: "New event" }));
    expect(form.getByLabelText("Starts")).toHaveValue("2026-11-26");
    await user.type(
      form.getByRole("textbox", { name: "Title" }),
      "Pay suppliers",
    );
    await user.selectOptions(
      form.getByRole("combobox", { name: "Repeat" }),
      "monthly",
    );
    await user.click(form.getByRole("button", { name: "Save" }));

    const card = within(
      await region.findByRole("article", { name: "Pay suppliers" }),
    );
    expect(card.getByText("Every month on day 26")).toBeInTheDocument();
    const saved = (
      await api.listEvents(
        at("2026-12-01T00:00:00Z"),
        at("2027-01-01T00:00:00Z"),
      )
    ).filter((e) => e.title === "Pay suppliers");
    expect(saved).toHaveLength(1);

    await user.click(region.getByRole("button", { name: "Next month" }));
    const december = within(
      await screen.findByRole("region", { name: "December 2026" }),
    );
    expect(
      await december.findByRole("button", { name: /Pay suppliers/ }),
    ).toBeInTheDocument();
  });

  it("checks the times before saving", async () => {
    const { user, region } = await openCalendar();
    await user.click(region.getByRole("button", { name: "New event" }));
    const form = within(region.getByRole("form", { name: "New event" }));
    await user.clear(form.getByLabelText("End Time"));
    await user.type(form.getByLabelText("End Time"), "00:30");
    await user.click(form.getByRole("button", { name: "Save" }));
    expect(await region.findByRole("alert")).toHaveTextContent(
      "The event ends before it starts.",
    );
  });

  it("deletes one date of a series, or the whole series", async () => {
    const { user, region } = await openCalendar();
    await user.click(
      day(region, /^Tue, Nov 10/).getByRole("button", {
        name: /Team stand-up/,
      }),
    );
    await user.click(region.getByRole("button", { name: "Delete" }));
    const dialog = within(
      screen.getByRole("dialog", { name: "Delete a repeating event" }),
    );
    await user.click(dialog.getByRole("button", { name: "Only this date" }));
    await waitFor(() => {
      expect(
        region.getByRole("gridcell", { name: /^Tue, Nov 10, no events$/ }),
      ).toBeInTheDocument();
    });
    expect(
      region.getByRole("gridcell", { name: /^Wed, Nov 11, 1 event$/ }),
    ).toBeInTheDocument();

    await user.click(
      day(region, /^Wed, Nov 11/).getByRole("button", {
        name: /Team stand-up/,
      }),
    );
    await user.click(region.getByRole("button", { name: "Delete" }));
    await user.click(
      within(screen.getByRole("dialog")).getByRole("button", {
        name: "Every date",
      }),
    );
    await waitFor(() => {
      expect(
        region.queryAllByRole("button", { name: /Team stand-up/ }),
      ).toHaveLength(0);
    });
  });

  it("edits a series from its first date, and Escape closes the editor", async () => {
    const { user, region } = await openCalendar();
    await user.click(
      day(region, /^Thu, Nov 12/).getByRole("button", {
        name: /Team stand-up/,
      }),
    );
    await user.click(region.getByRole("button", { name: "Edit" }));
    const form = within(
      await region.findByRole("form", { name: "Edit event" }),
    );
    expect(
      form.getByText("This event repeats. Changes apply to every date."),
    ).toBeInTheDocument();
    expect(form.getByLabelText("Starts")).toHaveValue("2026-11-09");
    expect(form.getByRole("combobox", { name: "Repeat" })).toHaveValue("daily");

    await user.keyboard("{Escape}");
    expect(
      region.queryByRole("form", { name: "Edit event" }),
    ).not.toBeInTheDocument();
    // The calendar itself is still open.
    expect(
      screen.getByRole("region", { name: "November 2026" }),
    ).toBeInTheDocument();
  });

  it("says when an account has no calendar on its server", async () => {
    const { region } = await openCalendar({
      calendarStatus: [
        { accountId: "work", state: "synced", message: null },
        {
          accountId: "personal",
          state: "unavailable",
          message: "The mail server doesn't offer a calendar.",
        },
      ],
    });
    const notes = within(region.getByRole("list", { name: "Calendar sync" }));
    expect(
      notes.getByText(
        /you@personal.example: The mail server doesn't offer a calendar./,
      ),
    ).toBeInTheDocument();
  });
});

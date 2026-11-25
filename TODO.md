# To do

Day-to-day Net2 work moving into Tusk, so Net2 itself is left for advanced
configuration: doors and controllers, timezones, access level definitions,
public and custom days, operators and lockdown rules.

## Done

- [x] **Move people to another department.** Manage tab: tick people,
      choose a department. `PUT /users/{id}/departments`, the same request
      Add people already sends on a real Net2.
- [x] **Mark leavers.** Manage tab: optional move to a leavers department,
      then `GET /users/{id}` and `PUT /users/{id}` with only the expiry
      changed to their last day. Access that already ends sooner is
      never extended.
- [x] **Year change.** Manage tab: renames the year departments from the
      top down (`PUT /departments/{id}`), then makes the new lowest year
      (`POST /departments`). A new run requires one department per year in
      the selected range. Tests cover every stopping point, including ranges
      with mixed `Y7` and `Year 8` naming.

## Check on a real Net2 before release

The browser smoke test exercises these workflows with a stand-in Net2.
Use a disposable Net2 database, a test person and test departments for the
release checks below.

Checked on a disposable Net2 database on 2026-11-25 (no doors configured):

- [x] `PUT /users/{id}` with the record from `GET /users/{id}` changes only
      the expiry: PIN, phone, custom fields, access levels, portrait,
      activation date, department and cards all stayed as they were.
- [x] `PUT /departments/{id}` with `{id, name}` renames the department, and
      its people stay in it.
- [x] `POST /departments` with `{name}` makes a department.
- [x] A full year change on test departments (Year 10 to 12), then run it
      again: nothing changed.
- [x] Undo a leaver: an explicit JSON `null` clears the expiry and keeps the
      rest of the record. Moving the expiry to a different date works.
- [x] Access levels: Net2 gives each person one access level. Sent
      several, it keeps the first and still answers 204. The Manage tab now
      sets or removes the one level and reads the set back.
- [x] Reviewed purge: the user and their cards are gone afterwards.
- [x] `sdk.UsersEx` has `LastKnownLocation`, not `LastArea`; the
      last-access query now uses it.

Still to check:

- [ ] Net2 stores expiry as a date only: `T23:59:00` comes back as
      `T00:00:00`. Check at a door whether a card still works on the last
      day and is refused the day after, then fix the "23:59" wording in the
      README and the Manage tab if needed.
- [ ] On a Net2 with doors: last-access and event fields, roll call, unknown
      cards, and open, hold and return to normal on a test door.
- [x] Replace a card on the Manage tab with the desk reader (2026-11-25):
      the new card is saved and the old one is marked lost.
- [x] Add people with several `;`-separated access levels: Net2 kept only
      the first (2026-11-25), so Tusk now holds such rows back.

## Implemented and browser-tested

- [x] **Undo a leaver.** Change or clear the expiry for ticked people.
- [x] **Replace one person's card.** Tick them on Manage, tap the new card,
      and confirm. Old cards are marked lost after Net2 saves the new card.
- [x] **Set or remove the access level** of ticked people while preserving
      individual door permissions.
- [x] **Show a person's site activity** using `sdk.UsersEx` and `GET /events`.
- [x] **Show fire roll call reports** and their entries from Net2.
- [x] **Assign a card tapped at a door** using `events/latestunknowntokens`.
- [x] **Open, hold open, or return a door to normal operation** from Manage.
- [x] **Purge ticked leavers past the retention period** after downloading
      the review and confirming a second time. Recheck each user before
      sending `DELETE /users/{id}?purge=true`.

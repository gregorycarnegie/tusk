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
      changed to 23:59 on their last day. Access that already ends sooner is
      never extended.
- [x] **Year change.** Manage tab: renames the year departments from the
      top down (`PUT /departments/{id}`), then makes the new lowest year
      (`POST /departments`). Running it again after a stop finishes without
      moving anyone twice (`src/manage.rs` tests every stopping point).

## Check on a real Net2 before release

Only the stand-in Net2 in `tests/static_site.py` has seen these. Use a test
person and test departments, not real ones.

- [ ] `PUT /users/{id}` with the record from `GET /users/{id}` changes only
      the expiry: PIN, custom fields, access levels, portrait, activation
      date and cards all stay as they were. If any are lost, the GET reply
      leaves them out and `with_expiry` must fetch and re-send them.
- [ ] The expiry shows as 23:59 on the chosen day in Net2, and a card is
      refused at a door the day after.
- [ ] `PUT /departments/{id}` with `{id, name}` renames the department, and
      its people stay in it.
- [ ] `POST /departments` with `{name}` makes a department.
- [ ] A full year change on a copy of the real department names, then run it
      again and see that nothing changes.

## Next, most useful first

- [ ] **Undo a leaver.** Clear or move the expiry date back from Tusk.
      Needs a live check of what clears an expiry (`null`, or leaving the
      field out, which the PUT-user trap says clears it).
- [ ] **Replacement card for one person.** Pick them on Manage, tap a card;
      reuse `save_card`, which already marks the old card lost.
- [ ] **Add or remove access levels for the ticked people,** for example
      sixth-form access after the year change. `PUT
      /users/{id}/doorpermissionset` replaces the whole set, so read it,
      change it, and send it back.
- [ ] **Was this person on site?** Their last access time and place
      (`sdk.UsersEx` through querydb) and recent events (`GET /events`).
      Read only.
- [ ] **Fire roll call.** `rollcallreports` endpoints. Read only.
- [ ] **Assign a card someone tapped at a door,** from
      `events/latestunknowntokens`, without the desk reader.
- [ ] **Open or hold open a door from reception.** `commands/door/open` and
      `holdopen`. Lockdown stays in Net2.
- [ ] **Delete leavers after a set number of years** (GDPR).
      `DELETE /users?purge=true` cannot be undone, so it needs a second
      confirmation and a results download first.

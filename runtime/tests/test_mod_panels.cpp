// SPDX-License-Identifier: GPL-3.0-or-later
// The code mods' panels (mod API version 4): the interface's layout, which EmblemWeave and ObeliskChamber share member for member,
// and add_panel.
#include <cstddef>
#include <string>

#include "nxrt/mod_api.h"
#include "nxrt/mod_panels.h"
#include "test_framework.h"

NXTEST_GROUP(mod_panels)

namespace modapi = nxrt::services::modapi;

// add_panel right after free_mem, panel_visible right after it; NxModUi a size and 14 functions in this order. A mod built against
// another runtime's header relies on every offset.
static_assert(offsetof(NxModApi, add_panel) == offsetof(NxModApi, free_mem) + sizeof(void*), "add_panel follows free_mem");
static_assert(offsetof(NxModApi, panel_visible) == offsetof(NxModApi, add_panel) + sizeof(void*), "panel_visible follows add_panel");
static_assert(sizeof(NxModApi) == offsetof(NxModApi, panel_visible) + sizeof(void*), "panel_visible is the last member");
#define UI_AT(member, n) static_assert(offsetof(NxModUi, member) == 8 + (n) * sizeof(void*), "NxModUi: " #member)
UI_AT(text, 0);
UI_AT(text_disabled, 1);
UI_AT(text_colored, 2);
UI_AT(checkbox, 3);
UI_AT(slider_float, 4);
UI_AT(slider_int, 5);
UI_AT(button, 6);
UI_AT(same_line, 7);
UI_AT(separator, 8);
UI_AT(begin_disabled, 9);
UI_AT(end_disabled, 10);
UI_AT(tree_node, 11);
UI_AT(tree_pop, 12);
UI_AT(tooltip, 13);
static_assert(sizeof(NxModUi) == 8 + 14 * sizeof(void*), "NxModUi ends at tooltip");
static_assert(NXMOD_API_VERSION == 4, "version 4 is exactly the panels: a later member needs a version agreed with the other runtimes");

namespace {
void panel(const NxModUi*, void*) {}
}  // namespace

NXTEST(add_panel_outside_a_mods_start) {
    static int user = 0;
    static const char kTitle[] = "Test panel (mod_panels)";
    // missing arguments: nothing added
    CHECK_EQ(modapi::add_panel(nullptr, &panel, &user), 0);
    CHECK_EQ(modapi::add_panel("", &panel, &user), 0);
    CHECK_EQ(modapi::add_panel(kTitle, nullptr, &user), 0);
    // not inside a mod's nxmod_early / nxmod_init: shown at once
    CHECK_EQ(modapi::add_panel(kTitle, &panel, &user), 1);
    CHECK(modapi::has_panels());
    int found = 0;
    for (const modapi::PanelInfo& p : modapi::panels()) {
        if (p.title != kTitle) continue;
        ++found;
        CHECK(p.fn == &panel);
        CHECK(p.user == &user);
        CHECK(p.mod.empty());
    }
    CHECK_EQ(found, 1);
}

NXTEST(panel_visible_follows_the_overlay) {
    modapi::set_panel_visible(true);
    CHECK_EQ(modapi::panel_visible(), 1);
    modapi::set_panel_visible(false);
    CHECK_EQ(modapi::panel_visible(), 0);
}

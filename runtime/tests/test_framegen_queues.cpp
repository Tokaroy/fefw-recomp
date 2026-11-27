// SPDX-License-Identifier: GPL-3.0-or-later
// Where the FSR 3 frame generation swapchain's queues come from (extras/framegen_ffx_queues.h). Pure data: each case is a device's queue
// families as Vulkan lists them.
#include "../src/gfx/extras/framegen_ffx_queues.h"

#include "test_framework.h"
NXTEST_GROUP(framegen_queues)

namespace {
using namespace nxrt::extras::fgq;

constexpr uint32_t kGfx = kGraphics | kCompute | kTransfer;
constexpr uint32_t kComp = kCompute | kTransfer;
constexpr uint32_t kVideoDecode = 0x20;

}  // namespace

// NVIDIA-like (16 graphics queues): all three in the game's family, as before
NXTEST(four_queues_in_the_game_family_keep_them_all_there) {
    const Plan p = plan({{kGfx, 16, true}, {kTransfer, 2, false}, {kComp, 8, true}}, 0);
    CHECK(p.on);
    CHECK(!p.cross_family);
    CHECK_EQ(p.present.family, 0u);
    CHECK_EQ(p.present.index, 1u);
    CHECK_EQ(p.acquire.index, 2u);
    CHECK_EQ(p.compute.index, 3u);
    CHECK_EQ(queues_in(p, 0, 0), 4u);
    CHECK_EQ(queues_in(p, 2, 0), 0u);
}

// RADV (Mesa): one graphics queue, compute queues that present
NXTEST(radv_takes_all_three_from_the_compute_family) {
    const Plan p = plan({{kGfx, 1, true}, {kComp, 4, true}, {kVideoDecode, 1, false}}, 0);
    CHECK(p.on);
    CHECK(p.cross_family);
    CHECK_EQ(p.present.family, 1u);
    CHECK_EQ(p.compute.family, 1u);
    CHECK_EQ(p.acquire.family, 1u);
    CHECK(p.present.index != p.compute.index);
    CHECK(p.present.index != p.acquire.index);
    CHECK(p.compute.index != p.acquire.index);
    CHECK_EQ(queues_in(p, 0, 0), 1u);
    CHECK_EQ(queues_in(p, 1, 0), 3u);
    CHECK_EQ(queues_in(p, 2, 0), 0u);   // video only: never used
}

NXTEST(no_queue_that_presents_is_off) {
    const Plan p = plan({{kGfx, 1, true}, {kComp, 4, false}}, 0);
    CHECK(!p.on);
    CHECK(!p.why.empty());
    CHECK_EQ(queues_in(p, 0, 0), 1u);
    CHECK_EQ(queues_in(p, 1, 0), 0u);
}

NXTEST(too_few_spare_queues_is_off) {
    const Plan p = plan({{kGfx, 1, true}, {kComp, 2, true}}, 0);
    CHECK(!p.on);
    CHECK(!p.why.empty());
}

// two spare graphics queues: present and acquire stay there, async compute goes to the compute family
NXTEST(spare_game_queues_are_used_before_other_families_except_compute) {
    const Plan p = plan({{kGfx, 3, true}, {kComp, 4, true}}, 0);
    CHECK(p.on);
    CHECK_EQ(p.present.family, 0u);
    CHECK_EQ(p.present.index, 1u);
    CHECK_EQ(p.acquire.family, 0u);
    CHECK_EQ(p.acquire.index, 2u);
    CHECK_EQ(p.compute.family, 1u);
    CHECK_EQ(p.compute.index, 0u);
    CHECK_EQ(queues_in(p, 0, 0), 3u);
    CHECK_EQ(queues_in(p, 1, 0), 1u);
}

NXTEST(protected_and_video_families_are_never_used) {
    const Plan p = plan({{kGfx, 1, true}, {kComp | kProtected, 8, true}, {kVideoDecode, 4, true}}, 0);
    CHECK(!p.on);
}

NXTEST(a_game_family_out_of_range_is_off) {
    const Plan p = plan({{kGfx, 16, true}}, 3);
    CHECK(!p.on);
}

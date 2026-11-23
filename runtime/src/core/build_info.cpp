// SPDX-License-Identifier: GPL-3.0-or-later
#include <string>

#include "nxrt/abi.h"
#include "nxrt/runtime.h"
namespace nxrt {
#define NXRT_STR2(x) #x
#define NXRT_STR(x) NXRT_STR2(x)
const char* abi_source() { return "recomp/include/nxrecomp/ctx.h, ABI version " NXRT_STR(NXRECOMP_ABI_VERSION); }
int abi_version() { return NXRECOMP_ABI_VERSION; }
namespace {
std::string g_app_build;
}
void set_app_build(const char* b) { g_app_build = b ? b : ""; }
const char* app_build() { return g_app_build.c_str(); }
}  // namespace nxrt

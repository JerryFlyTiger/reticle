// vcd_writer.cpp -- a small VCD waveform writer.
//
// The counterpart to vcd_summary.py in this directory: that one reads
// dumps, this one produces them. Useful when a C++ model needs to emit
// waveforms a normal viewer (GTKWave, Surfer, ...) can open.
//
//     c++ -std=c++17 -O2 -Wall -Wextra -o /tmp/vcd_writer vcd_writer.cpp
//     /tmp/vcd_writer > /tmp/demo.vcd
//     ../tools/vcd_summary.py /tmp/demo.vcd

#include <cstdint>
#include <iostream>
#include <map>
#include <ostream>
#include <sstream>
#include <stdexcept>
#include <string>
#include <vector>

namespace vcd {

// A VCD identifier is a short printable-ASCII code; the format assigns
// one per signal to keep the value-change section compact.
inline std::string identifier_for(std::size_t index) {
    static constexpr char kFirst = '!';
    static constexpr int kRange = '~' - '!' + 1;
    std::string out;
    do {
        out.push_back(static_cast<char>(kFirst + (index % kRange)));
        index /= kRange;
    } while (index > 0);
    return out;
}

class Writer {
public:
    Writer(std::ostream& out, std::string timescale)
        : out_(out), timescale_(std::move(timescale)) {}

    // Signals must all be declared before the first value change; the
    // format has a header section and a body, with no going back.
    int add_signal(const std::string& name, int width) {
        if (header_closed_) {
            throw std::logic_error("add_signal after the header was closed");
        }
        if (width < 1) {
            throw std::invalid_argument("signal width must be positive");
        }
        signals_.push_back({name, width, identifier_for(signals_.size())});
        return static_cast<int>(signals_.size()) - 1;
    }

    void set(int handle, std::uint64_t value) {
        if (handle < 0 || handle >= static_cast<int>(signals_.size())) {
            throw std::out_of_range("no such signal handle");
        }
        pending_[handle] = value;
    }

    // Emit everything queued since the last call, stamped at `time`.
    void tick(std::uint64_t time) {
        if (!header_closed_) {
            write_header();
        }
        if (pending_.empty()) {
            return;
        }
        out_ << '#' << time << '\n';
        for (const auto& [handle, value] : pending_) {
            const Signal& sig = signals_[static_cast<std::size_t>(handle)];
            if (sig.width == 1) {
                out_ << (value & 1u) << sig.ident << '\n';
            } else {
                out_ << 'b' << to_binary(value, sig.width) << ' ' << sig.ident
                     << '\n';
            }
        }
        pending_.clear();
    }

private:
    struct Signal {
        std::string name;
        int width;
        std::string ident;
    };

    static std::string to_binary(std::uint64_t value, int width) {
        std::string bits;
        for (int i = width - 1; i >= 0; --i) {
            bits.push_back(((value >> i) & 1u) ? '1' : '0');
        }
        // VCD wants the shortest form: leading zeros are implied.
        const std::size_t first_one = bits.find('1');
        return first_one == std::string::npos ? "0" : bits.substr(first_one);
    }

    void write_header() {
        out_ << "$timescale " << timescale_ << " $end\n";
        out_ << "$scope module demo $end\n";
        for (const Signal& sig : signals_) {
            out_ << "$var wire " << sig.width << ' ' << sig.ident << ' '
                 << sig.name << " $end\n";
        }
        out_ << "$upscope $end\n";
        out_ << "$enddefinitions $end\n";
        header_closed_ = true;
    }

    std::ostream& out_;
    std::string timescale_;
    std::vector<Signal> signals_;
    std::map<int, std::uint64_t> pending_;
    bool header_closed_ = false;
};

}  // namespace vcd

int main() {
    vcd::Writer writer(std::cout, "1ns");

    const int clk = writer.add_signal("clk", 1);
    const int rst_n = writer.add_signal("rst_n", 1);
    const int counter = writer.add_signal("counter", 8);
    const int overflow = writer.add_signal("overflow", 1);

    std::uint64_t count = 0;
    writer.set(clk, 0);
    writer.set(rst_n, 0);
    writer.set(counter, 0);
    writer.set(overflow, 0);
    writer.tick(0);

    for (std::uint64_t cycle = 1; cycle <= 40; ++cycle) {
        const std::uint64_t time = cycle * 10;

        writer.set(clk, 1);
        if (cycle == 3) {
            writer.set(rst_n, 1);
        }
        if (cycle > 3) {
            const std::uint64_t next = (count + 1) & 0xFFu;
            writer.set(overflow, next == 0 ? 1 : 0);
            count = next;
            writer.set(counter, count);
        }
        writer.tick(time);

        writer.set(clk, 0);
        writer.tick(time + 5);
    }

    return 0;
}

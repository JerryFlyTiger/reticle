#!/usr/bin/env perl
#
# simlog_report.pl -- collapse a simulator log into a report you can read.
#
# A failing regression prints the same handful of messages thousands of
# times; what you want is each distinct message once, with a count and
# the first place it happened. Text wrangling like this is the reason
# Perl is still installed on every EDA machine.
#
#     ./simlog_report.pl sim.log
#     ./simlog_report.pl --errors-only --context sim1.log sim2.log

use strict;
use warnings;
use Getopt::Long;

my $errors_only = 0;
my $show_context = 0;
my $max_rows = 25;

GetOptions(
    'errors-only' => \$errors_only,
    'context'     => \$show_context,
    'max=i'       => \$max_rows,
) or die "usage: $0 [--errors-only] [--context] [--max N] LOGFILE...\n";

@ARGV or die "usage: $0 [--errors-only] [--context] [--max N] LOGFILE...\n";

# Severity patterns, most severe first. Simulators disagree on the exact
# spelling, so each one gets a list of alternatives rather than a single
# regex nobody can extend later.
my @SEVERITIES = (
    [ FATAL   => qr/^\s*(?:\*\s*)?(?:UVM_FATAL|Fatal|FATAL)\b/ ],
    [ ERROR   => qr/^\s*(?:\*\s*)?(?:UVM_ERROR|Error|ERROR|\*E)\b/ ],
    [ WARNING => qr/^\s*(?:\*\s*)?(?:UVM_WARNING|Warning|WARNING|\*W)\b/ ],
);

my %seen;      # normalised message -> record
my %by_sev;    # severity -> count
my $lines = 0;

for my $path (@ARGV) {
    open my $fh, '<', $path or do {
        warn "$0: cannot read $path: $!\n";
        next;
    };

    while (my $line = <$fh>) {
        $lines++;
        chomp $line;

        my $severity;
        for my $rule (@SEVERITIES) {
            my ( $name, $re ) = @$rule;
            if ( $line =~ $re ) {
                $severity = $name;
                last;
            }
        }
        next unless defined $severity;
        next if $errors_only && $severity eq 'WARNING';

        $by_sev{$severity}++;

        # Normalise the parts that differ between otherwise identical
        # messages: timestamps, hex addresses, and bare numbers.
        my $key = $line;
        $key =~ s/\b\d+(?:\.\d+)?\s*(?:ns|ps|us|ms)\b/<TIME>/g;
        $key =~ s/\b0x[0-9a-fA-F]+\b/<HEX>/g;
        $key =~ s/\b\d+\b/<N>/g;

        if ( exists $seen{$key} ) {
            $seen{$key}{count}++;
        }
        else {
            $seen{$key} = {
                count    => 1,
                severity => $severity,
                sample   => $line,
                file     => $path,
                line_no  => $.,
            };
        }
    }

    close $fh;
}

my %rank = ( FATAL => 0, ERROR => 1, WARNING => 2 );

my @sorted = sort {
         $rank{ $seen{$a}{severity} } <=> $rank{ $seen{$b}{severity} }
      || $seen{$b}{count} <=> $seen{$a}{count}
      || $a cmp $b
} keys %seen;

printf "scanned %d line(s) in %d file(s)\n", $lines, scalar @ARGV;
printf "%s\n", join( '  ',
    map { sprintf '%s=%d', $_, $by_sev{$_} }
    grep { $by_sev{$_} } qw(FATAL ERROR WARNING) );

unless (@sorted) {
    print "\nclean -- no matching messages\n";
    exit 0;
}

print "\n";
my $shown = 0;
for my $key (@sorted) {
    last if $shown++ >= $max_rows;
    my $rec = $seen{$key};
    printf "%-7s x%-6d %s\n", $rec->{severity}, $rec->{count}, $rec->{sample};
    if ($show_context) {
        printf "%15s first seen at %s:%d\n", '', $rec->{file}, $rec->{line_no};
    }
}

my $hidden = @sorted - $shown;
printf "\n... and %d more distinct message(s)\n", $hidden if $hidden > 0;

# Non-zero exit if anything worse than a warning turned up, so this can
# gate a regression script.
exit( ( $by_sev{FATAL} || $by_sev{ERROR} ) ? 1 : 0 );

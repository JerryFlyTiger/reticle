// TimingReport.java -- pull the worst paths out of a static timing report.
//
// Timing reports are enormous and mostly boilerplate; what you want
// after a run is the worst slack per clock group and whether anything
// actually failed. This reads the common "Startpoint/Endpoint/slack"
// shape that most STA tools emit.
//
//     javac -d /tmp TimingReport.java
//     java -cp /tmp TimingReport report.rpt
//
// Exits 1 if any path has negative slack, so it can gate a flow script.

import java.io.BufferedReader;
import java.io.IOException;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.Comparator;
import java.util.List;
import java.util.regex.Matcher;
import java.util.regex.Pattern;

public final class TimingReport {

    /** One reported path. Slack is in nanoseconds; negative means failing. */
    static final class TimingPath {
        final String startpoint;
        final String endpoint;
        final String group;
        final double slack;

        TimingPath(String startpoint, String endpoint, String group, double slack) {
            this.startpoint = startpoint;
            this.endpoint = endpoint;
            this.group = group;
            this.slack = slack;
        }

        boolean failing() {
            return slack < 0.0;
        }

        @Override
        public String toString() {
            return String.format("%8.3f ns  %-10s %s -> %s",
                    slack, group, startpoint, endpoint);
        }
    }

    private static final Pattern STARTPOINT =
            Pattern.compile("^\\s*Startpoint:\\s*(\\S+)");
    private static final Pattern ENDPOINT =
            Pattern.compile("^\\s*Endpoint:\\s*(\\S+)");
    private static final Pattern GROUP =
            Pattern.compile("^\\s*Path Group:\\s*(\\S+)");
    private static final Pattern SLACK =
            Pattern.compile("^\\s*slack\\s*\\(\\w+\\)\\s*(-?\\d+\\.?\\d*)");

    static List<TimingPath> parse(BufferedReader reader) throws IOException {
        List<TimingPath> paths = new ArrayList<>();
        String startpoint = null;
        String endpoint = null;
        String group = "default";

        String line;
        while ((line = reader.readLine()) != null) {
            Matcher m = STARTPOINT.matcher(line);
            if (m.find()) {
                startpoint = m.group(1);
                continue;
            }
            m = ENDPOINT.matcher(line);
            if (m.find()) {
                endpoint = m.group(1);
                continue;
            }
            m = GROUP.matcher(line);
            if (m.find()) {
                group = m.group(1);
                continue;
            }
            m = SLACK.matcher(line);
            if (m.find() && startpoint != null && endpoint != null) {
                // A slack line closes the current path; reset so a
                // malformed block cannot inherit the previous one's
                // endpoints and report a path that was never in the file.
                paths.add(new TimingPath(startpoint, endpoint, group,
                        Double.parseDouble(m.group(1))));
                startpoint = null;
                endpoint = null;
            }
        }
        return paths;
    }

    public static void main(String[] args) throws IOException {
        if (args.length != 1) {
            System.err.println("usage: TimingReport REPORT_FILE");
            System.exit(2);
        }

        List<TimingPath> paths;
        try (BufferedReader reader = Files.newBufferedReader(
                Path.of(args[0]), StandardCharsets.UTF_8)) {
            paths = parse(reader);
        }

        if (paths.isEmpty()) {
            System.err.println("no timing paths recognised in " + args[0]);
            System.exit(1);
        }

        paths.sort(Comparator.comparingDouble(p -> p.slack));

        long failing = paths.stream().filter(TimingPath::failing).count();
        System.out.printf("%d path(s) parsed, %d failing%n", paths.size(), failing);
        System.out.println();

        int shown = Math.min(10, paths.size());
        System.out.println("worst " + shown + " path(s):");
        for (int i = 0; i < shown; i++) {
            System.out.println("  " + paths.get(i));
        }

        System.exit(failing > 0 ? 1 : 0);
    }

    private TimingReport() {
    }
}

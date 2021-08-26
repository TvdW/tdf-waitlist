import _ from "lodash";
import { Cell, CellHead, Row, Table, TableBody, TableHead } from "../../Components/Table";
import { useApi } from "../../api";
import { Badge } from "../../Components/Badge";

export function SkillHistory({ characterId }) {
  const [history] = useApi(`/api/history/skills?character_id=${characterId}`);

  if (!history) {
    return <em>Loading skill history...</em>;
  }

  var skillNames = _.invert(history.ids);

  var table = [];
  _.forEach(history.history, (historyLine) => {
    var variant =
      historyLine.old_level > historyLine.new_level
        ? "danger"
        : historyLine.old_level === historyLine.new_level
        ? ""
        : "success";
    table.push(
      <Row key={`${historyLine.skill_id} ${historyLine.logged_at}`}>
        <Cell>{new Date(historyLine.logged_at * 1000).toLocaleDateString()}</Cell>
        <Cell>{skillNames[historyLine.skill_id]}</Cell>
        <Cell>
          <Badge variant={variant}>
            {historyLine.old_level} → {historyLine.new_level}
          </Badge>
        </Cell>
      </Row>
    );
  });

  if (!table.length) {
    return <em>No skill history available</em>;
  }

  return (
    <Table fullWidth>
      <TableHead>
        <Row>
          <CellHead>Date</CellHead>
          <CellHead>Skill</CellHead>
          <CellHead></CellHead>
        </Row>
      </TableHead>
      <TableBody>{table}</TableBody>
    </Table>
  );
}

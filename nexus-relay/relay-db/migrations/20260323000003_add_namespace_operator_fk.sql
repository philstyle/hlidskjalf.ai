ALTER TABLE namespaces ADD CONSTRAINT fk_namespaces_operator
    FOREIGN KEY (operator_id) REFERENCES participants(id);
